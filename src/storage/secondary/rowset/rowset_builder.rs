// Copyright 2022 RisingLight Project Authors. Licensed under Apache-2.0.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use itertools::Itertools;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncWriteExt, BufWriter};

use super::super::{ColumnBuilderImpl, IndexBuilder};
use crate::array::DataChunk;
use crate::catalog::ColumnCatalog;
use crate::storage::secondary::ColumnBuilderOptions;
use crate::storage::StorageResult;

pub fn path_of_data_column(base: impl AsRef<Path>, column_info: &ColumnCatalog) -> PathBuf {
    path_of_column(base, column_info, ".col")
}

pub fn path_of_index_column(base: impl AsRef<Path>, column_info: &ColumnCatalog) -> PathBuf {
    path_of_column(base, column_info, ".idx")
}

pub fn path_of_column(
    base: impl AsRef<Path>,
    column_info: &ColumnCatalog,
    suffix: &str,
) -> PathBuf {
    base.as_ref()
        .join(format!("{}{}", column_info.id(), suffix))
}

/// Builds a Rowset from [`DataChunk`].
pub struct RowsetBuilder {
    /// Column information
    columns: Arc<[ColumnCatalog]>,

    /// Column data builders
    builders: Vec<ColumnBuilderImpl>,

    /// Output directory of the rowset
    directory: PathBuf,

    /// Count of rows in this rowset
    row_cnt: u32,

    /// Column builder options
    column_options: ColumnBuilderOptions,
}

impl RowsetBuilder {
    pub fn new(
        columns: Arc<[ColumnCatalog]>,
        directory: impl AsRef<Path>,
        column_options: ColumnBuilderOptions,
    ) -> Self {
        Self {
            builders: columns
                .iter()
                .map(|column| {
                    ColumnBuilderImpl::new_from_datatype(&column.datatype(), column_options.clone())
                })
                .collect_vec(),
            directory: directory.as_ref().to_path_buf(),
            columns,
            row_cnt: 0,
            column_options,
        }
    }

    pub fn append(&mut self, chunk: DataChunk) {
        self.row_cnt += chunk.cardinality() as u32;

        for idx in 0..chunk.column_count() {
            self.builders[idx].append(chunk.array_at(idx));
        }
    }

    async fn pipe_to_file(path: impl AsRef<Path>, data: Vec<u8>) -> StorageResult<()> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.as_ref())
            .await?;

        let mut writer = BufWriter::new(file);
        writer.write_all(&data).await?;
        writer.flush().await?;

        let file = writer.into_inner();
        file.sync_data().await?;

        Ok(())
    }

    async fn sync_dir(path: &impl AsRef<Path>) -> StorageResult<()> {
        File::open(path.as_ref()).await?.sync_data().await?;
        Ok(())
    }

    pub async fn finish_and_flush(self) -> StorageResult<()> {
        for (column_info, builder) in self.columns.iter().zip(self.builders) {
            let (index, data) = builder.finish();

            Self::pipe_to_file(path_of_data_column(&self.directory, column_info), data).await?;

            let mut index_builder =
                IndexBuilder::new(self.column_options.checksum_type, index.len());
            for index in index {
                index_builder.append(index);
            }

            Self::pipe_to_file(
                path_of_index_column(&self.directory, column_info),
                index_builder.finish(),
            )
            .await?;
        }

        Self::sync_dir(&self.directory).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::array::ArrayImpl;
    use crate::types::{DataTypeExt, DataTypeKind};

    #[tokio::test]
    async fn test_rowset_flush() {
        let tempdir = tempfile::tempdir().unwrap();

        let mut builder = RowsetBuilder::new(
            vec![ColumnCatalog::new(
                0,
                DataTypeKind::Int(None)
                    .nullable()
                    .to_column("v1".to_string()),
            )]
            .into(),
            tempdir.path(),
            ColumnBuilderOptions::default_for_test(),
        );

        for _ in 0..1000 {
            builder.append(
                [ArrayImpl::Int32(
                    [1, 2, 3].into_iter().cycle().take(1000).collect(),
                )]
                .into_iter()
                .collect(),
            )
        }

        builder.finish_and_flush().await.unwrap();
    }
}
