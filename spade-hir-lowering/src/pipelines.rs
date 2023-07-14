use std::collections::BTreeMap;

use local_impl::local_impl;
use mir::Register;
use mir::ValueName;
use mir::ValueNameSource;
use spade_common::{location_info::Loc, name::NameID};
use spade_diagnostics::diag_bail;
use spade_diagnostics::Diagnostic;
use spade_hir::expression::CallKind;
use spade_hir::Binding;
use spade_hir::TypeSpec;
use spade_hir::{ExprKind, Expression, Pattern, Statement};
use spade_mir as mir;

use crate::Context;
use crate::ExprLocal;
use crate::{error::Error, statement_list::StatementList, MirLowerable, NameIDExt, Result};

pub struct PipelineContext {
    /// Mapping from stage index to the corresponding enable signal, i.e. what
    /// `stage.ready` should map to. If the stage is unconditionally enabled,
    /// the corresponding value is `None`
    // NOTE: Current stage is being kept track of by [Context::Substitutions]
    pub ready_signals: Vec<Option<ValueName>>,
    /// Mapping from stage index to the corresponding valid signal. I.e. what
    /// `stage.valid` should map to. If the stage is always valid, the corresponding
    /// value is `None`
    pub valid_signals: Vec<Option<ValueName>>,
}

pub enum MaybePipelineContext {
    NotPipeline,
    Pipeline(PipelineContext),
}
impl MaybePipelineContext {
    /// Returns the pipeline context if we are in a pipeline, otherwise bails
    /// with a Diagnostic::bug on the specified Loc
    pub fn get<T>(&mut self, request_loc: &Loc<T>) -> Result<&mut PipelineContext> {
        match self {
            MaybePipelineContext::NotPipeline => {
                diag_bail!(request_loc, "Requesting pipeline context without pipeline")
            }
            MaybePipelineContext::Pipeline(ctx) => Ok(ctx),
        }
    }
}

pub fn handle_pattern(pat: &Pattern, live_vars: &mut Vec<NameID>) {
    // Add this variable to the live vars list
    for name in pat.get_names() {
        live_vars.push(name.inner.clone());
    }
}

pub fn handle_statement(
    statement: &Loc<Statement>,
    ctx: &mut Context,
    name_map: &mut BTreeMap<NameID, NameID>,
    statements: &mut StatementList,
    clock: &Loc<NameID>,
    local_conds: &mut Vec<Option<ValueName>>,
    stage_enable_names: &mut Vec<Option<ValueName>>,
    current_stage: &mut usize,
) -> Result<()> {
    match &statement.inner {
        Statement::Binding(Binding {
            pattern: pat,
            value: expr,
            wal_trace: _,
            ty: _,
        }) => {
            let time = expr.inner.kind.available_in()?;
            for name in pat.get_names() {
                let is_port = ctx
                    .types
                    .name_type(&name, ctx.symtab.symtab(), &ctx.item_list.types)?
                    .is_port();

                ctx.subs.set_available(name, time, is_port)
            }
        }
        Statement::Register(reg) => {
            let time = reg.inner.value.kind.available_in()?;
            for name in reg.pattern.get_names() {
                let is_port = ctx
                    .types
                    .name_type(&name, ctx.symtab.symtab(), &ctx.item_list.types)?
                    .is_port();
                ctx.subs.set_available(name, time, is_port)
            }
        }
        Statement::Declaration(_) => todo!(),
        Statement::PipelineRegMarker(cond) => {
            local_conds.push(if let Some(cond) = cond {
                statements.append(cond.lower(ctx)?);
                Some(cond.variable(ctx.subs)?)
            } else {
                None
            });
            let live_vars = ctx.subs.next_stage(ctx.symtab);

            // Generate pipeline regs for previous live vars
            for reg in &live_vars {
                if name_map
                    .insert(reg.new.clone(), reg.original.inner.clone())
                    .is_some()
                {
                    // NOTE: Panic because this should not occur in user code
                    panic!("inserted duplicate in name map");
                }

                let reg_type = ctx
                    .types
                    .name_type(&reg.original, ctx.symtab.symtab(), &ctx.item_list.types)?
                    .to_mir_type();
                // If this stage has an enable signal, generate a mux to optionally select
                // the previous value, otherwise use the previous value right away
                let next = if let Some(enable) = &stage_enable_names[*current_stage] {
                    let next_name = ValueName::Expr(ctx.idtracker.next());
                    statements.push_secondary(
                        mir::Statement::Binding(mir::Binding {
                            name: next_name.clone(),
                            operator: mir::Operator::Select,
                            operands: vec![
                                enable.clone(),
                                reg.previous.value_name(),
                                reg.new.value_name(),
                            ],
                            ty: reg_type.clone(),
                            loc: Some(statement.loc()),
                        }),
                        &reg.original,
                        "Pipeline enable mux",
                    );
                    next_name
                } else {
                    reg.previous.value_name()
                };

                statements.push_secondary(
                    mir::Statement::Register(mir::Register {
                        name: reg
                            .new
                            .value_name_with_alternate_source(ValueNameSource::Name(
                                reg.original.inner.clone(),
                            )),
                        ty: reg_type,
                        clock: clock.value_name(),
                        reset: None,
                        initial: None,
                        value: next,
                        traced: None,
                        // NOTE: Do we/can we also want to point to the declaration
                        // of the variable?
                        loc: Some(statement.loc()),
                    }),
                    &reg.original,
                    "Pipelined",
                );
            }
            *current_stage += 1;
        }
        Statement::Label(_) => {
            // Labels have no effect on codegen
        }
        Statement::Assert(_) => {
            // Assertions have no effect on pipeline state
        }
        Statement::WalSuffixed { .. } => {
            // Wal suffixes have no effect on pipeline state
        }
        Statement::Set { .. } => {
            // Set have no effect on pipeline state
        }
    }
    Ok(())
}

pub fn lower_pipeline<'a>(
    hir_inputs: &Vec<(Loc<NameID>, Loc<TypeSpec>)>,
    body: &Loc<Expression>,
    statements: &mut StatementList,
    ctx: &mut Context,
    // Map of names generated by codegen to the original name in the source code.
    name_map: &mut BTreeMap<NameID, NameID>,
) -> Result<()> {
    let clock = &hir_inputs[0].0;

    let (body_statements, _) = if let ExprKind::Block(block) = &body.kind {
        (&block.statements, &block.result)
    } else {
        panic!("Pipeline body was not a block");
    };

    for (name, _) in hir_inputs {
        let is_port = ctx
            .types
            .name_type(&name, ctx.symtab.symtab(), &ctx.item_list.types)?
            .is_port();

        ctx.subs.set_available(name.clone(), 0, is_port)
    }

    // If we have stage enable signals, we need to pre-allocate some variables
    // for the relevant stages, because the enable signal depends on downstream stages.
    // This builds a list of ValueNames which we need to fill in down the line, and
    // which will contain the enable signals.
    let mut stage_enable_names = vec![];
    let mut has_enable = false;
    for statement in body_statements.iter().rev() {
        match &statement.inner {
            Statement::PipelineRegMarker(cond) => {
                // Once we encounter the *last* reg statement with an enable, subsequent stages
                // have enables
                if cond.is_some() {
                    has_enable = true;
                }

                if has_enable {
                    stage_enable_names.push(Some(ValueName::Expr(ctx.idtracker.next())));
                } else {
                    stage_enable_names.push(None)
                }
            }
            _ => {}
        }
    }
    // We generated these in reverse order, so we need to reverse them back
    stage_enable_names.reverse();

    let mut current_stage = 0;
    let mut local_conds = vec![];
    for statement in body_statements {
        handle_statement(
            statement,
            ctx,
            name_map,
            statements,
            clock,
            &mut local_conds,
            &mut stage_enable_names,
            &mut current_stage,
        )?
    }

    // Codegen enable signals for the stages that need them. We need to generate them
    // in reverse order because upstream enables depend on downstream
    let mut current_enable = None;
    for (local_cond, enable_name) in local_conds.iter().zip(stage_enable_names.iter()).rev() {
        match (local_cond, &current_enable) {
            // First time we find a condition, alias it to the enable name for the current stage
            (Some(local), None) => {
                let name = enable_name
                    .clone()
                    .expect("No enable name for first stage that needs one");
                statements.push_anonymous(mir::Statement::Binding(mir::Binding {
                    name: name.clone(),
                    operator: mir::Operator::Alias,
                    operands: vec![local.clone()],
                    ty: mir::types::Type::Bool,
                    loc: None,
                }));
                current_enable = Some(name.clone());
            }
            (None, Some(prev)) => {
                let name = enable_name
                    .clone()
                    .expect("No enable name for first stage that needs one");
                // Since we have no new conditions, we can just alias the one from the previous
                // stage
                statements.push_anonymous(mir::Statement::Binding(mir::Binding {
                    name: name.clone(),
                    operator: mir::Operator::Alias,
                    operands: vec![prev.clone()],
                    ty: mir::types::Type::Bool,
                    loc: None,
                }));
                current_enable = Some(name.clone());
            }
            (Some(local), Some(prev)) => {
                let name = enable_name
                    .clone()
                    .expect("No enable name for first stage that needs one");
                statements.push_anonymous(mir::Statement::Binding(mir::Binding {
                    name: name.clone(),
                    operator: mir::Operator::LogicalAnd,
                    operands: vec![local.clone(), prev.clone()],
                    ty: mir::types::Type::Bool,
                    loc: None,
                }));
                current_enable = Some(name.clone());
            }
            (None, None) => {}
        }
    }

    // Codegen valid signals
    // The first stage, before any `reg` statement is valid, so we can initialize the vector
    // with `None`
    let mut valid_signals = vec![None];
    let mut last_cond: Option<ValueName> = None;
    for local_cond in local_conds {
        // Generate the conditions for validity of this stage
        let cond_name = match (local_cond, &last_cond) {
            // Both a local and a previous condition, or them together
            (Some(local), Some(prev)) => {
                let new_name = ValueName::Expr(ctx.idtracker.next());

                statements.push_anonymous(mir::Statement::Binding(mir::Binding {
                    name: new_name.clone(),
                    operator: mir::Operator::LogicalAnd,
                    operands: vec![local, prev.clone()],
                    ty: mir::types::Type::Bool,
                    loc: None,
                }));

                Some(new_name)
            }
            // New condition but no previous, alias
            (Some(local), None) => Some(local),
            (None, Some(prev)) => Some(prev.clone()),
            (None, None) => None,
        };
        // Register the local condition for one cycle.

        if let Some(cond_name) = cond_name {
            let new_name = ValueName::Expr(ctx.idtracker.next());

            statements.push_anonymous(mir::Statement::Register(Register {
                name: new_name.clone(),
                ty: mir::types::Type::Bool,
                clock: clock.value_name(),
                // FIXME: We should probably handle resets here, but I don't know how
                reset: None,
                initial: None,
                value: cond_name,
                traced: None,
                loc: None,
            }));

            last_cond = Some(new_name);
        }

        valid_signals.push(last_cond.clone());
    }

    let mut ready_signals = stage_enable_names.into_iter().collect::<Vec<_>>();
    // NOTE: The last stage needs a ready signal because you *can* use `stage.ready`
    // after the last `reg` in the final output expression, but it will be `None` because
    // there is no way to for it to be disabled
    ready_signals.push(None);
    *ctx.pipeline_context = MaybePipelineContext::Pipeline(PipelineContext {
        ready_signals,
        valid_signals,
    });

    Ok(())
}

/// Computes the time at which the specified expressions will be available. If there
/// is a mismatch, an error is returned
pub fn try_compute_availability(
    exprs: &[impl std::borrow::Borrow<Loc<Expression>>],
) -> Result<usize> {
    let mut result = None;
    for expr in exprs {
        let a = expr.borrow().kind.available_in()?;

        result = match result {
            None => Some(a),
            Some(prev) if a == prev => result,
            // NOTE: Safe index. This branch can only be reached in iteration 2 of the loop
            _ => {
                return Err(Error::AvailabilityMismatch {
                    prev: exprs[0].borrow().clone().map(|_| result.unwrap()),
                    new: expr.borrow().clone().map(|_| a),
                })
            }
        }
    }
    Ok(result.unwrap_or(0))
}

#[local_impl]
impl PipelineAvailability for ExprKind {
    fn available_in(&self) -> Result<usize> {
        match self {
            ExprKind::Identifier(_) => Ok(0),
            ExprKind::IntLiteral(_) => Ok(0),
            ExprKind::BoolLiteral(_) => Ok(0),
            ExprKind::BitLiteral(_) => Ok(0),
            ExprKind::CreatePorts => Ok(0),
            ExprKind::StageReady | ExprKind::StageValid => Ok(0),
            ExprKind::TupleLiteral(inner) => try_compute_availability(inner),
            ExprKind::ArrayLiteral(elems) => try_compute_availability(elems),
            ExprKind::Index(lhs, idx) => try_compute_availability(&[lhs.as_ref(), idx.as_ref()]),
            ExprKind::TupleIndex(lhs, _) => lhs.inner.kind.available_in(),
            ExprKind::FieldAccess(lhs, _) => lhs.inner.kind.available_in(),
            ExprKind::BinaryOperator(lhs, _, rhs) => {
                try_compute_availability(&[lhs.as_ref(), rhs.as_ref()])
            }
            ExprKind::UnaryOperator(_, val) => val.inner.kind.available_in(),
            ExprKind::Match(_, values) => {
                try_compute_availability(&values.iter().map(|(_, expr)| expr).collect::<Vec<_>>())
            }
            ExprKind::Block(inner) => {
                // NOTE: Do we want to allow delayed values inside blocks? That could lead to some
                // strange issues like
                // {
                //      let x = inst(10) subpipe();
                //      x // Will appear as having availability 1
                // }
                if let Some(result) = &inner.result {
                    result.kind.available_in()
                } else {
                    Ok(0)
                }
            }
            ExprKind::Call {
                kind: CallKind::Pipeline(_, depth),
                ..
            } => {
                // FIXME: Re-add this check to allow nested pipelines
                // let arg_availability = try_compute_availability(
                //     &args.iter().map(|arg| &arg.value).collect::<Vec<_>>(),
                // )?;
                Ok(depth.inner as usize)
            }
            ExprKind::Call {
                kind: CallKind::Function,
                ..
            }
            | ExprKind::Call {
                kind: CallKind::Entity(_),
                ..
            } => Ok(0),
            ExprKind::If(_, t, f) => try_compute_availability(&[t.as_ref(), f.as_ref()]),
            ExprKind::PipelineRef { .. } => Ok(0),
            ExprKind::MethodCall { name, .. } => diag_bail!(
                name,
                "Method call should already have been lowered by this point"
            ),
            ExprKind::Null => {
                panic!("Null expression during pipeline lowering")
            }
        }
    }
}
