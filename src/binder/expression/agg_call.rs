// Copyright 2022 RisingLight Project Authors. Licensed under Apache-2.0.

use std::fmt::Formatter;

use serde::Serialize;

use super::*;
use crate::binder::{BindError, Binder, BoundExpr};
use crate::parser::{BinaryOperator, FunctionArg, FunctionArgExpr};
use crate::types::{DataType, DataTypeKind};

/// Aggregation kind
#[derive(Debug, PartialEq, Clone, Serialize)]
pub enum AggKind {
    Avg,
    RowCount,
    Max,
    Min,
    Sum,
    Count,
}

impl std::fmt::Display for AggKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use AggKind::*;
        write!(
            f,
            "{}",
            match self {
                Avg => "avg",
                RowCount | Count => "count",
                Max => "max",
                Min => "min",
                Sum => "sum",
            }
        )
    }
}

/// Represents an aggregation function
#[derive(PartialEq, Clone, Serialize)]
pub struct BoundAggCall {
    pub kind: AggKind,
    pub args: Vec<BoundExpr>,
    pub return_type: DataType,
    // TODO: add distinct keyword
}

impl std::fmt::Debug for BoundAggCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?}({:?}) -> {:?}",
            self.kind, self.args, self.return_type
        )
    }
}

impl Binder {
    pub fn bind_function(&mut self, func: &Function) -> Result<BoundExpr, BindError> {
        // TODO: Support scalar function
        let mut args = Vec::new();
        for arg in &func.args {
            let arg = match &arg {
                FunctionArg::Named { arg, .. } => arg,
                FunctionArg::Unnamed(arg) => arg,
            };
            match arg {
                FunctionArgExpr::Expr(expr) => args.push(self.bind_expr(expr)?),
                FunctionArgExpr::Wildcard => {
                    // No argument in row count
                    args.clear();
                    break;
                }
                _ => todo!("Support aggregate argument: {:?}", arg),
            }
        }
        let (kind, return_type) = match func.name.to_string().to_lowercase().as_str() {
            "avg" => (
                AggKind::Avg,
                Some(DataType::new(DataTypeKind::Double, false)),
            ),
            "count" => {
                if args.is_empty() {
                    for ref_id in self.context.regular_tables.values() {
                        let table = self.catalog.get_table(ref_id).unwrap();
                        if let Some(col) = table.get_column_by_id(0) {
                            let column_ref_id = ColumnRefId::from_table(*ref_id, col.id());
                            self.record_regular_table_column(
                                &table.name(),
                                col.name(),
                                col.id(),
                                col.desc().clone(),
                            );
                            let expr = BoundExpr::ColumnRef(BoundColumnRef {
                                table_name: table.name(),
                                column_ref_id,
                                is_primary_key: col.is_primary(),
                                desc: col.desc().clone(),
                            });
                            args.push(expr);
                            break;
                        }
                    }
                    (
                        AggKind::RowCount,
                        Some(DataType::new(DataTypeKind::Int(None), false)),
                    )
                } else {
                    (
                        AggKind::Count,
                        Some(DataType::new(DataTypeKind::Int(None), false)),
                    )
                }
            }
            "max" => (AggKind::Max, args[0].return_type()),
            "min" => (AggKind::Min, args[0].return_type()),
            "sum" => (AggKind::Sum, args[0].return_type()),
            _ => panic!("Unsupported function: {}", func.name),
        };

        match kind {
            // Rewrite `avg` into `sum / count`
            AggKind::Avg => Ok(BoundExpr::BinaryOp(BoundBinaryOp {
                op: BinaryOperator::Divide,
                left_expr: Box::new(BoundExpr::AggCall(BoundAggCall {
                    kind: AggKind::Sum,
                    args: args.clone(),
                    return_type: args[0].return_type().unwrap(),
                })),
                right_expr: Box::new(BoundExpr::TypeCast(BoundTypeCast {
                    ty: args[0].return_type().unwrap().kind(),
                    expr: Box::new(BoundExpr::AggCall(BoundAggCall {
                        kind: AggKind::Count,
                        args,
                        return_type: DataType::new(DataTypeKind::Int(None), false),
                    })),
                })),
                return_type,
            })),
            _ => Ok(BoundExpr::AggCall(BoundAggCall {
                kind,
                args,
                return_type: return_type.unwrap(),
            })),
        }
    }
}
