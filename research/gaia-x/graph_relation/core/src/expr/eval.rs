//
//! Copyright 2020 Alibaba Group Holding Limited.
//!
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at
//!
//! http://www.apache.org/licenses/LICENSE-2.0
//!
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.
//!

use crate::error::{ParsePbError, ParsePbResult};
use crate::expr::error::{ExprError, ExprResult};
use crate::generated::common as pb;
use crate::generated::common::expr_unit::Item;
use crate::generated::common::{Arithmetic, ExprUnit, Logical};
use crate::graph::element::Element;
use crate::graph::property::{Details, PropKey};
use crate::{FromPb, NameOrId};
use dyn_type::arith::Exp;
use dyn_type::{BorrowObject, Object};
use std::cell::RefCell;

pub struct Evaluator<'a> {
    /// A suffix-tree-based expression for evaluating
    suffix_tree: Vec<InnerOpr>,
    /// A stack for evaluating the suffix-tree-based expression
    /// Wrap it in a `RefCell` to avoid conflict mutable reference
    stack: RefCell<Vec<BorrowObject<'a>>>,
}

/// An inner representation of `pb::ExprUnit` for one-shot translation of `pb::ExprUnit`.
enum InnerOpr {
    Logical(pb::Logical),
    Arith(pb::Arithmetic),
    Const(Option<Object>),
    Var {
        tag: NameOrId,
        prop_key: Option<PropKey>,
    },
}

/// A `Context` gives the behavior of obtaining a certain tag from the runtime
/// for evaluating variables in an expression.
pub trait Context<E: Element> {
    fn get(&self, _tag: &NameOrId) -> Option<&E> {
        None
    }
}

pub struct NoneContext {}

impl Context<()> for NoneContext {}

impl<'a> FromPb<Vec<pb::ExprUnit>> for Evaluator<'a> {
    fn from_pb(suffix_tree: Vec<ExprUnit>) -> ParsePbResult<Self>
    where
        Self: Sized,
    {
        let mut inner_tree: Vec<InnerOpr> = Vec::with_capacity(suffix_tree.len());
        for unit in suffix_tree {
            inner_tree.push(InnerOpr::from_pb(unit)?);
        }
        Ok(Self {
            suffix_tree: inner_tree,
            stack: RefCell::new(vec![]),
        })
    }
}

fn apply_arith<'a>(
    arith: &pb::Arithmetic,
    first: Option<BorrowObject<'a>>,
    second: Option<BorrowObject<'a>>,
) -> ExprResult<BorrowObject<'a>> {
    if first.is_some() && second.is_some() {
        let a = first.unwrap();
        let b = second.unwrap();
        Ok(match arith {
            Arithmetic::Add => BorrowObject::Primitive(a.as_primitive()? + b.as_primitive()?),
            Arithmetic::Sub => BorrowObject::Primitive(a.as_primitive()? - b.as_primitive()?),
            Arithmetic::Mul => BorrowObject::Primitive(a.as_primitive()? * b.as_primitive()?),
            Arithmetic::Div => BorrowObject::Primitive(a.as_primitive()? / b.as_primitive()?),
            Arithmetic::Mod => BorrowObject::Primitive(a.as_primitive()? % b.as_primitive()?),
            Arithmetic::Exp => BorrowObject::Primitive(a.as_primitive()?.exp(b.as_primitive()?)),
        })
    } else {
        Err(ExprError::MissingOperands)
    }
}

fn apply_logical<'a>(
    logical: &pb::Logical,
    first: Option<BorrowObject<'a>>,
    second: Option<BorrowObject<'a>>,
) -> ExprResult<BorrowObject<'a>> {
    if logical == &Logical::Not {
        if let Some(a) = first {
            return Ok((!a.as_bool()?).into());
        }
    } else {
        if first.is_some() && second.is_some() {
            let a = first.unwrap();
            let b = second.unwrap();
            let rst = match logical {
                Logical::Eq => (a == b).into(),
                Logical::Ne => (a != b).into(),
                Logical::Lt => (a < b).into(),
                Logical::Le => (a <= b).into(),
                Logical::Gt => (a > b).into(),
                Logical::Ge => (a >= b).into(),
                Logical::And => (a.as_bool()? && b.as_bool()?).into(),
                Logical::Or => (a.as_bool()? || b.as_bool()?).into(),
                Logical::Not => unreachable!(),
                // todo within, without
                _ => unimplemented!(),
            };
            return Ok(rst);
        }
    }

    Err(ExprError::MissingOperands)
}

// Private api
impl<'a> Evaluator<'a> {
    /// Evaluate simple expression that contains less than three operators
    /// without using the stack.
    fn eval_without_stack<E: Element, C: Context<E>>(
        &'a self,
        context: Option<&C>,
    ) -> ExprResult<Object> {
        assert!(self.suffix_tree.len() <= 3);
        if self.suffix_tree.is_empty() {
            return Err(ExprError::EmptyExpression);
        } else if self.suffix_tree.len() == 1 {
            return Ok(self.suffix_tree[0]
                .eval_as_borrow_object(context)?
                .ok_or(ExprError::NoneOperand)?
                .into());
        } else if self.suffix_tree.len() == 2 {
            // must be not
            if let InnerOpr::Logical(logical) = &self.suffix_tree[1] {
                return Ok(apply_logical(
                    logical,
                    self.suffix_tree[0].eval_as_borrow_object(context)?,
                    None,
                )?
                .into());
            }
        } else {
            if let InnerOpr::Logical(logical) = &self.suffix_tree[2] {
                return Ok(apply_logical(
                    logical,
                    self.suffix_tree[0].eval_as_borrow_object(context)?,
                    self.suffix_tree[1].eval_as_borrow_object(context)?,
                )?
                .into());
            } else if let InnerOpr::Arith(arith) = &self.suffix_tree[2] {
                return Ok(apply_arith(
                    arith,
                    self.suffix_tree[0].eval_as_borrow_object(context)?,
                    self.suffix_tree[1].eval_as_borrow_object(context)?,
                )?
                .into());
            }
        }

        Err("invalid expression".into())
    }
}

impl<'a> Evaluator<'a> {
    /// Reset the status of the evaluator for further evaluation
    pub fn reset(&self) {
        self.stack.borrow_mut().clear();
    }

    /// Evaluate an expression with an optional context.
    pub fn eval<E: Element + 'a, C: Context<E> + 'a>(
        &'a self,
        context: Option<&'a C>,
    ) -> ExprResult<Object> {
        let mut stack = self.stack.borrow_mut();
        if self.suffix_tree.len() <= 3 {
            return self.eval_without_stack(context);
        }
        stack.clear();
        for opr in &self.suffix_tree {
            if opr.is_operand() {
                if let Some(obj) = opr.eval_as_borrow_object(context)? {
                    stack.push(obj);
                } else {
                    return Err(ExprError::NoneOperand);
                }
            } else {
                let first = stack.pop();
                match opr {
                    InnerOpr::Logical(logical) => {
                        let rst = if logical == &Logical::Not {
                            apply_logical(logical, first, None)?
                        } else {
                            apply_logical(logical, stack.pop(), first)?
                        };
                        stack.push(rst);
                    }
                    InnerOpr::Arith(arith) => {
                        let rst = apply_arith(arith, stack.pop(), first)?;
                        stack.push(rst);
                    }
                    _ => unreachable!(),
                }
            }
        }

        if stack.len() == 1 {
            Ok(stack.pop().unwrap().into())
        } else {
            Err("invalid expression".into())
        }
    }
}

impl FromPb<pb::ExprUnit> for InnerOpr {
    fn from_pb(unit: ExprUnit) -> ParsePbResult<Self>
    where
        Self: Sized,
    {
        if let Some(item) = unit.item {
            let result = match item {
                Item::Logical(logical) => {
                    Self::Logical(unsafe { std::mem::transmute::<_, pb::Logical>(logical) })
                }
                Item::Arith(arith) => {
                    Self::Arith(unsafe { std::mem::transmute::<_, pb::Arithmetic>(arith) })
                }
                Item::Const(c) => Self::Const(c.into_object()?),
                Item::Var(var) => {
                    let tag = NameOrId::from_pb(var.tag.unwrap())?;
                    if let Some(property) = var.property {
                        Self::Var {
                            tag,
                            prop_key: Some(PropKey::from_pb(property)?),
                        }
                    } else {
                        Self::Var {
                            tag,
                            prop_key: None,
                        }
                    }
                }
            };
            Ok(result)
        } else {
            Err(ParsePbError::from("empty value provided"))
        }
    }
}

impl InnerOpr {
    pub fn eval_as_borrow_object<'a, E: Element + 'a, C: Context<E> + 'a>(
        &'a self,
        context: Option<&'a C>,
    ) -> ExprResult<Option<BorrowObject<'a>>> {
        match self {
            Self::Const(c_opt) => Ok(if let Some(opt) = c_opt {
                Some(opt.as_borrow())
            } else {
                None
            }),
            Self::Var { tag, prop_key } => {
                if context.is_some() {
                    let ctxt = context.unwrap();
                    if let Some(property) = prop_key {
                        if let Some(element) = ctxt.get(tag) {
                            if let Some(details) = element.details() {
                                return Ok(details.get(property));
                            }
                        }
                    } else {
                        if let Some(field) = ctxt.get(tag) {
                            return Ok(Some(field.as_borrow_object()));
                        }
                    }
                }

                Err(ExprError::MissingContext(
                    "missing context for evaluating variables".into(),
                ))
            }
            _ => Ok(None),
        }
    }

    pub fn is_operand(&self) -> bool {
        match self {
            InnerOpr::Const(_) | InnerOpr::Var { .. } => true,
            _ => false,
        }
    }
}

impl pb::Const {
    pub fn into_object(self) -> ParsePbResult<Option<Object>> {
        use pb::value::Item::*;
        if let Some(val) = &self.value {
            if let Some(item) = val.item.as_ref() {
                return match item {
                    Boolean(b) => Ok(Some((*b).into())),
                    I32(i) => Ok(Some((*i).into())),
                    I64(i) => Ok(Some((*i).into())),
                    F64(f) => Ok(Some((*f).into())),
                    Str(s) => Ok(Some(s.clone().into())),
                    Blob(blob) => Ok(Some(blob.clone().into())),
                    None(_) => Ok(Option::None),
                    I32Array(_) | I64Array(_) | F64Array(_) | StrArray(_) => {
                        Err(ParsePbError::from("the const values of `I32Array`, `I64Array`, `F64Array`, `StrArray` are unsupported"))
                    }
                };
            }
        }

        Err(ParsePbError::from("empty value provided"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::to_suffix_expr_pb;
    use crate::expr::token::tokenize;

    #[test]
    fn test_eval_simple() {
        let cases: Vec<&str> = vec![
            "7 + 3",          // 10
            "7.0 + 3",        // 10.0
            "7 * 3",          // 21
            "7 / 3",          // 2
            "7 ^ 3",          // 343
            "7 ^ -3",         // 1 / 343
            "7 % 3",          // 1
            "7 -3",           // 4
            "-3 + 7",         // 4
            "-3",             // -3
            "-3.0",           // -3.0
            "false",          // false
            "!true",          // false
            "!10",            // false
            "!0",             // true
            "true || true",   // true
            "true || false",  // true
            "false || false", // false
            "true && true",   // true
            "true && false",  // false
            "1 > 2",          // false
            "1 < 2",          // true
            "1 >= 2",         // false
            "2 <= 2",         // true,
            "2 == 2",         // true
            "1.0 > 2.0",      // false
        ];

        let expected: Vec<Object> = vec![
            Object::from(10),
            Object::from(10.0),
            Object::from(21),
            Object::from(2),
            Object::from(343),
            Object::from(1.0 / 343.0),
            Object::from(1),
            Object::from(4),
            Object::from(4),
            Object::from(-3),
            Object::from(-3.0),
            Object::from(false),
            Object::from(false),
            Object::from(false),
            Object::from(true),
            Object::from(true),
            Object::from(true),
            Object::from(false),
            Object::from(true),
            Object::from(false),
            Object::from(false),
            Object::from(true),
            Object::from(false),
            Object::from(true),
            Object::from(true),
            Object::from(false),
        ];

        for (case, expected) in cases.into_iter().zip(expected.into_iter()) {
            let eval =
                Evaluator::from_pb(to_suffix_expr_pb(tokenize(case).unwrap()).unwrap()).unwrap();
            assert_eq!(eval.eval::<(), NoneContext>(None).unwrap(), expected);
        }
    }

    #[test]
    fn test_eval_complex() {
        let cases: Vec<&str> = vec![
            "(-10)",                                 // -10
            "2 * 2 - 3",                             // 1
            "2 * (2 - 3)",                           // -2
            "6 / 2 - 3",                             // 0
            "6 / (2 - 3)",                           // -6
            "2 * 1e-3",                              // 0.002
            "1 > 2 && 1 < 3",                        // false
            "1 > 2 || 1 < 3",                        // true
            "2 ^ 10 > 10",                           // true
            "2 / 5 ^ 2",                             // 0
            "2.0 / 5 ^ 2",                           // 2.0 / 25
            "((1 + 2) * 3) / (7 * 8) + 12.5 / 10.1", // 1.2376237623762376
            "((1 + 2) * 3) / 7 * 8 + 12.5 / 10.1",   // 9.237623762376238
            "((1 + 2) * 3) / 7 * 8 + 12.5 / 10.1 \
                == ((1 + 2) * 3) / (7 * 8) + 12.5 / 10.1", // false
        ];

        let expected: Vec<Object> = vec![
            Object::from(-10),
            Object::from(1),
            Object::from(-2),
            Object::from(0),
            Object::from(-6),
            Object::from(0.002),
            Object::from(false),
            Object::from(true),
            Object::from(true),
            Object::from(0),
            Object::from(2.0 / 25.0),
            Object::from(1.2376237623762376),
            Object::from(9.237623762376238),
            Object::from(false),
        ];

        for (case, expected) in cases.into_iter().zip(expected.into_iter()) {
            let eval =
                Evaluator::from_pb(to_suffix_expr_pb(tokenize(case).unwrap()).unwrap()).unwrap();
            assert_eq!(eval.eval::<(), NoneContext>(None).unwrap(), expected);
        }
    }
}
