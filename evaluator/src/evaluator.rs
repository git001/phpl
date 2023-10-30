use std::rc::Rc;
use std::{fs, str};

use php_parser_rs::parser::ast::identifiers::Identifier;
use php_parser_rs::parser::ast::operators::{
    BitwiseOperationExpression, ComparisonOperationExpression, LogicalOperationExpression,
};

use php_parser_rs::{
    lexer::token::Span,
    parser::ast::{
        arguments::Argument,
        literals::Literal,
        operators::{ArithmeticOperationExpression, AssignmentOperationExpression},
        variables::Variable,
        Expression, Statement,
    },
};

use crate::helpers::parse_php_file;
use crate::{
    environment::Environment,
    helpers::get_variable_span,
    php_value::{ErrorLevel, PhpError, PhpValue},
};

const NULL: PhpValue = PhpValue::Null;

pub struct Evaluator {
    /// The output of the evaluated code
    pub output: String,

    /// Whether the PHP code is currently "open"
    php_open: bool,

    /// Whether the PHP code must die
    pub die: bool,

    /// The environment of the code
    pub env: Environment,

    pub warnings: Vec<PhpError>,

    pub included_files: Vec<String>,
    pub required_files: Vec<String>,
}

impl Evaluator {
    pub fn new() -> Evaluator {
        Evaluator {
            output: String::new(),
            php_open: false,
            die: false,
            env: Environment::new(),
            warnings: vec![],
            included_files: vec![],
            required_files: vec![],
        }
    }

    /// Returns a new child evaluator based on the current evaluator.
    ///
    /// This is only used with include and require statements.
    pub fn new_child(&self) -> Evaluator {
        Evaluator {
            output: String::new(),
            php_open: false,
            die: false,
            env: self.env.clone(),
            warnings: vec![],
            included_files: vec![],
            required_files: vec![],
        }
    }

    pub fn eval_statement(&mut self, node: Statement) -> Result<PhpValue, PhpError> {
        match node {
            Statement::FullOpeningTag(_) => {
                self.php_open = true;

                Ok(NULL)
            }
            Statement::ClosingTag(_) => {
                self.php_open = false;

                Ok(NULL)
            }
            Statement::InlineHtml(html) => {
                self.output += html.html.to_string().as_str();

                Ok(NULL)
            }
            Statement::Expression(e) => {
                let expression_result = self.eval_expression(e.expression);

                if expression_result.is_err() {
                    return Err(expression_result.unwrap_err());
                }

                Ok(NULL)
            }
            Statement::Echo(echo) => {
                for expr in echo.values {
                    let expression_result = self.eval_expression(expr)?;

                    let expression_as_string = expression_result.to_string();

                    if expression_as_string.is_none() {
                        self.warnings.push(PhpError {
                            level: ErrorLevel::Warning,
                            message: format!(
                                "{} to string conversion failed.",
                                expression_result.clone().get_type()
                            ),
                            line: echo.echo.line,
                        });

                        self.output += expression_result.get_type().as_str();
                    }

                    self.output += expression_as_string.unwrap().as_str();
                }

                Ok(NULL)
            }
            _ => {
                println!("TODO: statement {:#?}\n", node);
                Ok(NULL)
            }
        }
    }

    fn eval_expression(&mut self, expr: Expression) -> Result<PhpValue, PhpError> {
        match expr {
            Expression::Eval(_) => todo!(),
            Expression::Empty(ee) => {
                let arg = ee.argument.argument;

                match arg {
                    Argument::Named(na) => Err(PhpError {
                        level: ErrorLevel::ParseError,
                        message: "Named arguments are not supported in empty()".to_string(),
                        line: na.colon.line,
                    }),

                    Argument::Positional(pa) => {
                        let arg_as_php_value = self.eval_expression(pa.value)?;

                        let match_result = match arg_as_php_value {
                            PhpValue::Null => PhpValue::Bool(true),
                            PhpValue::Bool(b) => {
                                if !b {
                                    PhpValue::Bool(true);
                                }

                                PhpValue::Bool(false)
                            }
                            _ => PhpValue::Bool(false),
                        };

                        Ok(match_result)
                    }
                }
            }
            Expression::Die(_) => {
                self.die();

                Ok(NULL)
            }
            Expression::Exit(_) => {
                self.die();

                Ok(NULL)
            }
            Expression::Isset(ie) => {
                let args = ie.arguments.arguments;

                let mut args_values: Vec<PhpValue> = Vec::new();

                for arg in args {
                    match arg {
                        Argument::Named(na) => {
                            return Err(PhpError {
                                level: ErrorLevel::ParseError,
                                message: "Named arguments are not supported in isset()".to_string(),
                                line: na.colon.line,
                            });
                        }
                        Argument::Positional(pa) => {
                            let arg_as_php_value = self.eval_expression(pa.value)?;

                            args_values.push(arg_as_php_value);
                        }
                    }
                }

                for arg_value in args_values {
                    match arg_value {
                        PhpValue::Null => {
                            PhpValue::Bool(false);
                        }
                        _ => {
                            continue;
                        }
                    }
                }

                Ok(PhpValue::Bool(true))
            }
            Expression::Unset(ue) => {
                let args = ue.arguments;

                for arg in args {
                    match arg {
                        Argument::Named(arg) => {
                            return Err(PhpError {
                                level: ErrorLevel::ParseError,
                                message: "Named arguments are not supported in unset()".to_string(),
                                line: arg.colon.line,
                            });
                        }
                        Argument::Positional(pa) => {
                            if let Expression::Variable(va) = pa.value {
                                let var_name = self.get_variable_name(va)?;

                                // delete the variable from the environment
                                self.env.delete_var(var_name.as_str());
                            } else {
                                return Err(PhpError {
                                    level: ErrorLevel::ParseError,
                                    message: "Only variables can be unset".to_string(),
                                    line: ue.unset.line,
                                });
                            }
                        }
                    }
                }

                Ok(NULL)
            }
            Expression::Print(pe) => {
                if pe.value.is_some() {
                    let value = self.eval_expression(*pe.value.unwrap())?;

                    let value_as_string = value.to_string();

                    if value_as_string.is_none() {
                        self.warnings.push(PhpError {
                            level: ErrorLevel::Warning,
                            message: format!(
                                "{} to string conversion failed.",
                                value.clone().get_type()
                            ),
                            line: pe.print.line,
                        });

                        return Ok(PhpValue::String(value.get_type()));
                    }

                    self.output += value_as_string.unwrap().as_str();
                } else if pe.argument.is_some() {
                    let arg = *pe.argument.unwrap();

                    match arg.argument {
                        Argument::Positional(pa) => {
                            let value = self.eval_expression(pa.value)?;

                            let value_as_string = value.to_string();

                            if value_as_string.is_none() {
                                self.warnings.push(PhpError {
                                    level: ErrorLevel::Warning,
                                    message: format!(
                                        "{} to string conversion failed.",
                                        value.clone().get_type()
                                    ),
                                    line: pe.print.line,
                                });

                                return Ok(PhpValue::String(value.get_type()));
                            }

                            self.output += value_as_string.unwrap().as_str();
                        }
                        _ => {
                            return Err(PhpError {
                                level: ErrorLevel::ParseError,
                                message: "Only positional arguments are supported in print()"
                                    .to_string(),
                                line: pe.print.line,
                            });
                        }
                    }
                }

                Ok(NULL)
            }
            Expression::Literal(l) => match l {
                Literal::String(s) => {
                    let string = s.value.to_string();

                    Ok(PhpValue::String(string))
                }
                Literal::Integer(i) => {
                    let str_value = str::from_utf8(i.value.as_ref()).unwrap();

                    let int_value: i32 = str_value.parse().unwrap();

                    Ok(PhpValue::Int(int_value))
                }
                Literal::Float(f) => {
                    let str_value = str::from_utf8(f.value.as_ref()).unwrap();

                    let float_value: f32 = str_value.parse().unwrap();

                    Ok(PhpValue::Float(float_value))
                }
            },
            Expression::ArithmeticOperation(operation) => match operation {
                ArithmeticOperationExpression::Addition { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    self.php_value_or_die(left_value + right_value)
                }
                ArithmeticOperationExpression::Subtraction { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    self.php_value_or_die(left_value - right_value)
                }
                ArithmeticOperationExpression::Multiplication { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    self.php_value_or_die(left_value * right_value)
                }
                ArithmeticOperationExpression::Division { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    self.php_value_or_die(left_value / right_value)
                }
                ArithmeticOperationExpression::Modulo { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    self.php_value_or_die(left_value % right_value)
                }
                ArithmeticOperationExpression::Exponentiation { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    self.php_value_or_die(left_value.pow(right_value))
                }
                ArithmeticOperationExpression::Negative { right, .. } => {
                    let right_value = self.eval_expression(*right)?;

                    self.php_value_or_die(right_value * PhpValue::Int(-1))
                }
                ArithmeticOperationExpression::Positive { right, .. } => {
                    let right_value = self.eval_expression(*right)?;

                    self.php_value_or_die(right_value * PhpValue::Int(1))
                }
                ArithmeticOperationExpression::PreIncrement { right, .. } => {
                    let right_value = self.eval_expression(*right)?;

                    self.php_value_or_die(PhpValue::Int(1) + right_value)
                }
                ArithmeticOperationExpression::PostIncrement { left, .. } => {
                    let left_value = self.eval_expression(*left)?;

                    self.php_value_or_die(left_value + PhpValue::Int(1))
                }
                ArithmeticOperationExpression::PreDecrement { right, .. } => {
                    let right_value = self.eval_expression(*right)?;

                    self.php_value_or_die(right_value - PhpValue::Int(1))
                }
                ArithmeticOperationExpression::PostDecrement { left, .. } => {
                    let left_value = self.eval_expression(*left)?;

                    self.php_value_or_die(left_value - PhpValue::Int(1))
                }
            },
            Expression::AssignmentOperation(operation) => match operation {
                AssignmentOperationExpression::Assign {
                    left,
                    equals,
                    right,
                } => {
                    let Expression::Variable(left_var) = *left else {
						return Err(PhpError {
							level: ErrorLevel::ParseError,
							message: "Only variables can be assigned".to_string(),
							line: equals.line
						});
					};

                    let left_var_name = self.get_variable_name(left_var)?;

                    if let Expression::Reference(reference) = *right {
                        let Expression::Variable(right_var) = *reference.right else {
							return Err(PhpError {
								level: ErrorLevel::ParseError,
								message: "References must be to variables".to_string(),
								line: reference.ampersand.line
							});
						};

                        let right_var_name = self.get_variable_name(right_var)?;

                        if !self.env.var_exists(&right_var_name) {
                            self.env.set_var(&right_var_name, NULL)
                        }

                        let cloned_env = self.env.clone();

                        let right_value = cloned_env.get_var_with_rc(&right_var_name).unwrap();

                        self.env.set_var_rc(&left_var_name, Rc::clone(right_value));

                        return Ok(right_value.borrow().clone());
                    } else {
                        let right_value = self.eval_expression(*right)?;

                        let right_value_clone = right_value.clone();

                        if !self.env.var_exists(&left_var_name) {
                            self.env.set_var(&left_var_name, right_value_clone);
                        } else {
                            let old_value = self.env.get_var_with_rc(&left_var_name).unwrap();

                            *old_value.borrow_mut() = right_value_clone;
                        }

                        Ok(right_value)
                    }
                }
                AssignmentOperationExpression::Addition {
                    left,
                    plus_equals,
                    right,
                } => self.change_var_value(*left, plus_equals, *right, "+"),
                AssignmentOperationExpression::Subtraction {
                    left,
                    minus_equals,
                    right,
                } => self.change_var_value(*left, minus_equals, *right, "-"),
                AssignmentOperationExpression::Multiplication {
                    left,
                    asterisk_equals,
                    right,
                } => self.change_var_value(*left, asterisk_equals, *right, "*"),
                AssignmentOperationExpression::Division {
                    left,
                    slash_equals,
                    right,
                } => self.change_var_value(*left, slash_equals, *right, "/"),
                AssignmentOperationExpression::Modulo {
                    left,
                    percent_equals,
                    right,
                } => self.change_var_value(*left, percent_equals, *right, "%"),
                AssignmentOperationExpression::Exponentiation {
                    left,
                    pow_equals,
                    right,
                } => self.change_var_value(*left, pow_equals, *right, "**"),
                AssignmentOperationExpression::Concat {
                    left,
                    dot_equals,
                    right,
                } => self.change_var_value(*left, dot_equals, *right, "."),
                AssignmentOperationExpression::BitwiseAnd {
                    left,
                    ampersand_equals,
                    right,
                } => self.change_var_value(*left, ampersand_equals, *right, "&"),
                AssignmentOperationExpression::BitwiseOr {
                    left,
                    pipe_equals,
                    right,
                } => self.change_var_value(*left, pipe_equals, *right, "|"),
                AssignmentOperationExpression::BitwiseXor {
                    left,
                    caret_equals,
                    right,
                } => self.change_var_value(*left, caret_equals, *right, "^"),
                AssignmentOperationExpression::LeftShift {
                    left,
                    left_shift_equals,
                    right,
                } => self.change_var_value(*left, left_shift_equals, *right, "<<"),
                AssignmentOperationExpression::RightShift {
                    left,
                    right_shift_equals,
                    right,
                } => self.change_var_value(*left, right_shift_equals, *right, ">>"),
                AssignmentOperationExpression::Coalesce {
                    left,
                    coalesce_equals,
                    right,
                } => self.change_var_value(*left, coalesce_equals, *right, "??"),
            },
            Expression::BitwiseOperation(operation) => match operation {
                BitwiseOperationExpression::And { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    self.php_value_or_die(left_value & right_value)
                }
                BitwiseOperationExpression::Or { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    self.php_value_or_die(left_value | right_value)
                }
                BitwiseOperationExpression::Xor { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    self.php_value_or_die(left_value ^ right_value)
                }
                BitwiseOperationExpression::LeftShift { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    self.php_value_or_die(left_value << right_value)
                }
                BitwiseOperationExpression::RightShift { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    self.php_value_or_die(left_value >> right_value)
                }
                BitwiseOperationExpression::Not { right, .. } => {
                    let right_value = self.eval_expression(*right)?;

                    self.php_value_or_die(!right_value)
                }
            },
            Expression::ComparisonOperation(operation) => match operation {
                ComparisonOperationExpression::Equal { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    Ok(PhpValue::Bool(left_value == right_value))
                }
                ComparisonOperationExpression::Identical { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    if left_value.clone().get_type() != right_value.clone().get_type() {
                        PhpValue::Bool(false);
                    }

                    Ok(PhpValue::Bool(left_value == right_value))
                }
                ComparisonOperationExpression::NotEqual { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    Ok(PhpValue::Bool(left_value != right_value))
                }
                ComparisonOperationExpression::AngledNotEqual { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    Ok(PhpValue::Bool(left_value != right_value))
                }
                ComparisonOperationExpression::NotIdentical { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    if left_value.clone().get_type() != right_value.clone().get_type() {
                        PhpValue::Bool(true);
                    }

                    Ok(PhpValue::Bool(left_value != right_value))
                }
                ComparisonOperationExpression::LessThan { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    Ok(PhpValue::Bool(left_value < right_value))
                }
                ComparisonOperationExpression::GreaterThan { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    Ok(PhpValue::Bool(left_value > right_value))
                }
                ComparisonOperationExpression::LessThanOrEqual { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    Ok(PhpValue::Bool(left_value <= right_value))
                }
                ComparisonOperationExpression::GreaterThanOrEqual { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    Ok(PhpValue::Bool(left_value >= right_value))
                }
                ComparisonOperationExpression::Spaceship { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    if left_value < right_value {
                        Ok(PhpValue::Int(-1))
                    } else if left_value > right_value {
                        Ok(PhpValue::Int(1))
                    } else {
                        Ok(PhpValue::Int(0))
                    }
                }
            },
            Expression::LogicalOperation(operation) => match operation {
                LogicalOperationExpression::And { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    Ok(PhpValue::Bool(
                        left_value.is_true() && right_value.is_true(),
                    ))
                }
                LogicalOperationExpression::Or { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    Ok(PhpValue::Bool(
                        left_value.is_true() || right_value.is_true(),
                    ))
                }
                LogicalOperationExpression::Not { right, .. } => {
                    let right_value = self.eval_expression(*right)?;

                    Ok(PhpValue::Bool(!right_value.is_true()))
                }
                LogicalOperationExpression::LogicalAnd { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    Ok(PhpValue::Bool(
                        left_value.is_true() && right_value.is_true(),
                    ))
                }
                LogicalOperationExpression::LogicalOr { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    Ok(PhpValue::Bool(
                        left_value.is_true() || right_value.is_true(),
                    ))
                }
                LogicalOperationExpression::LogicalXor { left, right, .. } => {
                    let left_value = self.eval_expression(*left)?;
                    let right_value = self.eval_expression(*right)?;

                    Ok(PhpValue::Bool(left_value.is_true() ^ right_value.is_true()))
                }
            },
            Expression::Concat(expression) => {
                let left_value = self.eval_expression(*expression.left)?;
                let right_value = self.eval_expression(*expression.right)?;

                self.php_value_or_die(left_value.concat(right_value))
            }
            Expression::Instanceof(instanceof) => {
                let Expression::Variable(left_expr) = *instanceof.left else {
                    let error =
                        "Left side of instanceof must be a variable".to_string();

                    return Err(PhpError { level: ErrorLevel::Fatal, message: error, line: instanceof.instanceof.line });
				};

                let left_expr_value = self.get_variable_value(left_expr)?;

                let PhpValue::Object(left_object) = left_expr_value else {
					let error =
						format!(
							"Left side of instanceof must be an object, got {}",
							left_expr_value.get_type()
						);

					return Err(PhpError { level: ErrorLevel::Fatal, message: error, line: instanceof.instanceof.line });
				};

                let right_expr_value = self.eval_expression(*instanceof.right)?;

                let is_instance_of = left_object.is_instance_of(right_expr_value);

                match is_instance_of {
                    Ok(value) => Ok(PhpValue::Bool(value)),
                    Err(error) => self.eval_error(error),
                }
            }
            Expression::Reference(reference) => {
                let error = format!("Unexpected reference expression",);

                Err(PhpError {
                    level: ErrorLevel::ParseError,
                    message: error,
                    line: reference.ampersand.line,
                })
            }
            Expression::Parenthesized(parenthesized) => self.eval_expression(*parenthesized.expr),
            Expression::ErrorSuppress(error_expression) => {
                let old_php_die = self.die;
                let old_warnings = self.warnings.clone();

                self.eval_expression(*error_expression.expr)?;

                if old_php_die != self.die || old_warnings.len() != self.warnings.len() {
                    self.die = old_php_die;
                    self.warnings = old_warnings;
                }

                Ok(NULL)
            }
            Expression::Identifier(identifier) => match identifier {
                Identifier::SimpleIdentifier(simple_identifier) => {
                    let identifier_name = &simple_identifier.value.to_string();

                    let expr = self.env.get_identifier(identifier_name);

                    if expr.is_some() {
                        Ok(expr.unwrap())
                    } else {
                        let error = format!("Identifier {} not found", identifier_name,);

                        Err(PhpError {
                            level: ErrorLevel::Fatal,
                            message: error,
                            line: simple_identifier.span.line,
                        })
                    }
                }
                _ => todo!(),
            },
            Expression::Variable(var) => self.get_var(var),
            Expression::Include(include) => {
                self.handle_include(*include.path, false, include.include)
            }
            Expression::IncludeOnce(include) => {
                self.handle_include(*include.path, true, include.include_once)
            }
            Expression::Require(require) => {
                self.handle_require(*require.path, false, require.require)
            }
            Expression::RequireOnce(require) => {
                self.handle_require(*require.path, true, require.require_once)
            }
            Expression::Bool(b) => Ok(PhpValue::Bool(b.value)),
            _ => Ok(NULL),
        }
    }

    /*
     * Private functions
     */

    fn die(&mut self) {
        self.die = true;
    }

    fn php_value_or_die(
        &mut self,
        value: Result<PhpValue, PhpError>,
    ) -> Result<PhpValue, PhpError> {
        match value {
            Ok(value) => Ok(value),
            Err(error) => Err(error),
        }
    }

    fn get_variable_name(&mut self, variable: Variable) -> Result<String, PhpError> {
        match variable {
            Variable::SimpleVariable(sv) => Ok(sv.name.to_string()),
            Variable::VariableVariable(vv) => {
                let value = self.get_variable_value(*vv.variable)?;

                if let PhpValue::String(value) = value {
                    Ok(value)
                } else {
                    let error = format!(
                        "Variable variable must be a string, got {}",
                        value.get_type(),
                    );

                    Err(PhpError {
                        level: ErrorLevel::Fatal,
                        message: error,
                        line: vv.span.line,
                    })
                }
            }
            Variable::BracedVariableVariable(bvv) => {
                let expr_value = self.eval_expression(*bvv.variable)?;

                let expr_as_string = expr_value.to_string();

                if expr_as_string.is_none() {
                    self.warnings.push(PhpError {
                        level: ErrorLevel::Warning,
                        message: format!("{} to string conversion failed", expr_value.get_type(),),
                        line: bvv.start.line,
                    });

                    self.warnings.push(PhpError {
                        level: ErrorLevel::Warning,
                        message: format!("Undefined variable $ on line {}", bvv.start.line),
                        line: bvv.start.line,
                    });

                    return Ok("".to_string());
                }

                let variable_name = expr_as_string.unwrap();

                Ok(variable_name)
            }
        }
    }

    fn get_variable_value(&mut self, variable: Variable) -> Result<PhpValue, PhpError> {
        match variable {
            Variable::SimpleVariable(sv) => {
                let var_name = sv.name.to_string();

                let value = self.env.get_var(&var_name);

                if value.is_some() {
                    Ok(value.unwrap())
                } else {
                    let warning =
                        format!("Undefined variable {} on line {}", var_name, sv.span.line);

                    self.warnings.push(PhpError {
                        level: ErrorLevel::Warning,
                        message: warning,
                        line: sv.span.line,
                    });

                    Ok(NULL)
                }
            }
            Variable::VariableVariable(vv) => self.get_var(*vv.variable),
            Variable::BracedVariableVariable(bvv) => {
                let expr_value = self.eval_expression(*bvv.variable)?;

                let expr_as_string = expr_value.to_string();

                if expr_as_string.is_none() {
                    self.warnings.push(PhpError {
                        level: ErrorLevel::Warning,
                        message: format!(
                            "Braced variable variable must be a string, got {}",
                            expr_value.get_type(),
                        ),
                        line: bvv.start.line,
                    });

                    self.warnings.push(PhpError {
                        level: ErrorLevel::Warning,
                        message: "Undefined variable $".to_string(),
                        line: bvv.start.line,
                    });

                    return Ok(NULL);
                }

                let variable_name = expr_as_string.unwrap();

                if !self.env.var_exists(&variable_name) {
                    self.warnings.push(PhpError {
                        level: ErrorLevel::Warning,
                        message: format!("Undefined variable $ on line {}", bvv.start.line),
                        line: bvv.start.line,
                    });

                    return Ok(NULL);
                }

                Ok(self.env.get_var(&variable_name).unwrap())
            }
        }
    }

    fn change_var_value(
        &mut self,
        left: Expression,
        span: Span,
        right: Expression,
        operation: &str,
    ) -> Result<PhpValue, PhpError> {
        let right_value = self.eval_expression(right)?;

        let Expression::Variable(var) = left else {
            return Err(PhpError {
				level: ErrorLevel::ParseError,
				message: "Only variables can be assigned".to_string(),
				line: span.line,
			});
        };

        let var_name = self.get_variable_name(var)?;

        let current_var_value = self.env.get_var(&var_name);

        if current_var_value.is_none() {
            let error = format!("Undefined variable {}", var_name);

            return Err(PhpError {
                level: ErrorLevel::Fatal,
                message: error,
                line: span.line,
            });
        }

        let current_var_value = current_var_value.unwrap();

        let new_value = match operation {
            "+" => self.php_value_or_die(current_var_value + right_value),
            "-" => self.php_value_or_die(current_var_value - right_value),
            "*" => self.php_value_or_die(current_var_value * right_value),
            "/" => self.php_value_or_die(current_var_value / right_value),
            "%" => self.php_value_or_die(current_var_value % right_value),
            "**" => self.php_value_or_die(current_var_value.pow(right_value)),
            "." => self.php_value_or_die(current_var_value.concat(right_value)),
            "&" => self.php_value_or_die(current_var_value & right_value),
            "|" => self.php_value_or_die(current_var_value | right_value),
            "^" => self.php_value_or_die(current_var_value ^ right_value),
            "<<" => self.php_value_or_die(current_var_value << right_value),
            ">>" => self.php_value_or_die(current_var_value >> right_value),
            "??" => {
                if current_var_value.is_null() {
                    Ok(right_value)
                } else {
                    Ok(current_var_value)
                }
            }
            _ => Ok(NULL),
        }?;

        let new_value_clone = new_value.clone();

        if !self.env.var_exists(&var_name) {
            self.env.set_var(&var_name, new_value_clone);
        } else {
            let old_value = self.env.get_var_with_rc(&var_name).unwrap();

            *old_value.borrow_mut() = new_value_clone;
        }

        Ok(new_value)
    }

    /// Returns the value of the variable. If it does not exist, the warning is added and Null is returned.
    fn get_var(&mut self, variable: Variable) -> Result<PhpValue, PhpError> {
        let var_name = self.get_variable_name(variable.clone())?;

        let value = self.env.get_var(&var_name);

        if value.is_some() {
            Ok(value.unwrap())
        } else {
            let warning = format!("Undefined variable {}", var_name,);

            self.warnings.push(PhpError {
                level: ErrorLevel::Warning,
                message: warning,
                line: get_variable_span(variable).line,
            });

            Ok(NULL)
        }
    }

    fn eval_error(&mut self, error: PhpError) -> Result<PhpValue, PhpError> {
        match error.level {
            ErrorLevel::Fatal => Err(error),
            ErrorLevel::Warning => {
                self.warnings.push(error);

                Ok(NULL)
            }
            ErrorLevel::ParseError => Err(error),
            ErrorLevel::Raw => Err(error),
        }
    }

    fn handle_include(
        &mut self,
        path: Expression,
        once: bool,
        span: Span,
    ) -> Result<PhpValue, PhpError> {
        let path = self.eval_expression(path)?;

        let path_as_string = path.to_string();

        if path_as_string.is_none() {
            self.warnings.push(PhpError {
                level: ErrorLevel::Warning,
                message: format!("{} to string conversion failed", path.get_type(),),
                line: span.line,
            });
        }

        let real_path = path_as_string.unwrap_or("".to_string());

        if real_path.is_empty() {
            let error = format!("Path cannot be empty");

            return Err(PhpError {
                level: ErrorLevel::Fatal,
                message: error,
                line: span.line,
            });
        }

        if once && self.included_files.iter().any(|i| *i == real_path) {
            return Ok(PhpValue::Bool(true));
        }

        let content = fs::read_to_string(real_path.clone());

        if content.is_err() {
            let fn_name = if once { "include_once" } else { "include" };

            let warning = PhpError {
                level: ErrorLevel::Warning,
                message: format!(
                    "{}({}): Failed to open stream: {}",
                    fn_name,
                    real_path,
                    content.unwrap_err()
                ),
                line: span.line,
            };

            self.warnings.push(warning);

            return Ok(NULL);
        }

        self.included_files.push(real_path.clone());

        parse_php_file(self, &real_path, &content.unwrap())
    }

    fn handle_require(
        &mut self,
        path: Expression,
        once: bool,
        span: Span,
    ) -> Result<PhpValue, PhpError> {
        let path = self.eval_expression(path)?;

        let path_as_string = path.to_string();

        if path_as_string.is_none() {
            self.warnings.push(PhpError {
                level: ErrorLevel::Warning,
                message: format!("{} to string conversion failed", path.get_type(),),
                line: span.line,
            });
        }

        let real_path = path_as_string.unwrap_or("".to_string());

        if real_path.is_empty() {
            let error = format!("Path cannot be empty");

            return Err(PhpError {
                level: ErrorLevel::Fatal,
                message: error,
                line: span.line,
            });
        }

        if once && self.required_files.iter().any(|i| *i == real_path) {
            return Ok(PhpValue::Bool(true));
        }

        let content = fs::read_to_string(real_path.clone());

        if content.is_err() {
            let fn_name = if once { "require_once" } else { "require" };

            let error = PhpError {
                level: ErrorLevel::Fatal,
                message: format!(
                    "{}({}): Failed to open stream: {}",
                    fn_name,
                    real_path,
                    content.unwrap_err()
                ),
                line: span.line,
            };

            self.warnings.push(error);

            return Ok(NULL);
        }

        self.required_files.push(real_path.clone());

        parse_php_file(self, &real_path, &content.unwrap())
    }
}
