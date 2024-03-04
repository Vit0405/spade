// This algorithm is based off the excellent lecture here
// https://www.youtube.com/watch?v=xJXcZp2vgLs
//
// The basic idea is to go through every node in the HIR tree, for every typed thing,
// add an equation indicating a constraint on that thing. This can onlytydone once
// and should be done by the visitor for that node. The visitor should then unify
// types according to the rules of the node.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use colored::Colorize;
use hir::{Binding, Parameter, UnitHead, WalTrace};
use itertools::Itertools;
use num::{BigInt, Zero};
use serde::{Deserialize, Serialize};
use spade_common::num_ext::InfallibleToBigInt;
use spade_diagnostics::Diagnostic;
use spade_macros::trace_typechecker;
use trace_stack::TraceStack;
use tracing::{info, trace};

use spade_common::location_info::{Loc, WithLocation};
use spade_common::name::{Identifier, NameID, Path};
use spade_hir as hir;
use spade_hir::param_util::{match_args_with_params, Argument};
use spade_hir::symbol_table::{Patternable, PatternableKind, SymbolTable, TypeSymbol};
use spade_hir::{
    ArgumentList, Block, ExprKind, Expression, ItemList, Pattern, PatternArgument, Register,
    Statement, TypeParam, Unit,
};
use spade_types::KnownType;

use constraints::{
    bits_to_store, ce_int, ce_var, ConstraintExpr, ConstraintRhs, ConstraintSource, TypeConstraints,
};
use equation::{TraitList, TraitReq, TypeEquations, TypeVar, TypedExpression};
use error::{
    error_pattern_type_mismatch, Result, UnificationError, UnificationErrorExt, UnificationTrace,
};
use fixed_types::{t_bool, t_clock, t_int, t_uint};
use requirements::{Replacement, Requirement};
use trace_stack::{format_trace_stack, TraceStackEntry};

use crate::error::TypeMismatch as Tm;
use crate::fixed_types::t_void;
use crate::requirements::ConstantInt;

mod constraints;
pub mod dump;
pub mod equation;
pub mod error;
pub mod expression;
pub mod fixed_types;
pub mod method_resolution;
pub mod mir_type_lowering;
mod requirements;
pub mod testutil;
pub mod trace_stack;

pub struct Context<'a> {
    pub symtab: &'a SymbolTable,
    pub items: &'a ItemList,
}

// NOTE(allow) This is a debug macro which is not normally used but can come in handy
#[allow(unused_macros)]
macro_rules! add_trace {
    ($self:expr, $($arg : tt) *) => {
        $self.trace_stack.push(TraceStack::Message(format!($($arg)*)))
    }
}

#[derive(Debug)]
pub enum GenericListSource<'a> {
    /// For when you just need a new generic list but have no need to refer back
    /// to it in the future
    Anonymous,
    /// For managing the generics of callable with the specified name when type checking
    /// their body
    Definition(&'a NameID),
    /// For expressions which instantiate generic items
    Expression(u64),
}

/// Stored version of GenericListSource
#[derive(Clone, Hash, Eq, PartialEq, Debug)]
pub enum GenericListToken {
    Anonymous(usize),
    Definition(NameID),
    Expression(u64),
}

pub struct TurbofishCtx<'a> {
    turbofish: &'a Loc<Vec<Loc<hir::TypeExpression>>>,
    prev_generic_list: &'a GenericListToken,
    type_ctx: &'a Context<'a>,
}

/// State of the type inference algorithm
#[derive(Clone)]
pub struct TypeState {
    equations: TypeEquations,
    next_typeid: u64,
    // List of the mapping between generic parameters and type vars.
    // The key is the index of the expression for which this generic list is associated. (if this
    // is a generic list for a call whose expression id is x to f<A, B>, then generic_lists[x] will
    // be {A: <type var>, b: <type var>}
    // Managed here because unification must update *all* TypeVars in existence.
    generic_lists: HashMap<GenericListToken, HashMap<NameID, TypeVar>>,

    constraints: TypeConstraints,

    /// Requirements which must be fulfilled but which do not guide further type inference.
    /// For example, if seeing `let y = x.a` before knowing the type of `x`, a requirement is
    /// added to say "x has field a, and y should be the type of that field"
    requirements: Vec<Requirement>,

    replacements: HashMap<TypeVar, TypeVar>,

    pub trace_stack: Arc<TraceStack>,

    /// (Experimental) Use Affine- or Interval-Arithmetic to bounds check integers in a separate
    /// module.
    pub use_wordlenght_inference: bool,
}

impl Default for TypeState {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeState {
    pub fn new() -> Self {
        Self {
            equations: HashMap::new(),
            next_typeid: 0,
            trace_stack: Arc::new(TraceStack::new()),
            constraints: TypeConstraints::new(),
            requirements: vec![],
            replacements: HashMap::new(),
            generic_lists: HashMap::new(),
            use_wordlenght_inference: false,
        }
    }

    pub fn set_wordlength_inferece(self, use_wordlenght_inference: bool) -> Self {
        Self {
            use_wordlenght_inference,
            ..self
        }
    }

    pub fn get_equations(&self) -> &TypeEquations {
        &self.equations
    }

    pub fn get_constraints(&self) -> &TypeConstraints {
        &self.constraints
    }

    // Get a generic list with a safe unwrap since a token is acquired
    pub fn get_generic_list<'a>(
        &'a self,
        generic_list_token: &'a GenericListToken,
    ) -> &'a HashMap<NameID, TypeVar> {
        &self.generic_lists[&generic_list_token]
    }

    #[tracing::instrument(level = "trace", skip_all)]
    fn hir_type_expr_to_var<'a>(
        &'a self,
        e: &Loc<hir::TypeExpression>,
        generic_list_token: &GenericListToken,
    ) -> TypeVar {
        match &e.inner {
            hir::TypeExpression::Integer(i) => {
                TypeVar::Known(e.loc(), KnownType::Integer(i.clone()), vec![])
            }
            hir::TypeExpression::TypeSpec(spec) => {
                self.type_var_from_hir(e.loc(), &spec.clone(), generic_list_token)
            }
        }
    }

    #[tracing::instrument(level = "trace", skip_all, fields(%hir_type))]
    pub fn type_var_from_hir<'a>(
        &'a self,
        loc: Loc<()>,
        hir_type: &crate::hir::TypeSpec,
        generic_list_token: &GenericListToken,
    ) -> TypeVar {
        let generic_list = self.get_generic_list(generic_list_token);
        match &hir_type {
            hir::TypeSpec::Declared(base, params) => {
                let params = params
                    .iter()
                    .map(|e| self.hir_type_expr_to_var(e, generic_list_token))
                    .collect();

                TypeVar::Known(loc, KnownType::Named(base.inner.clone()), params)
            }
            hir::TypeSpec::Generic(name) => match generic_list.get(&name.inner) {
                Some(t) => t.clone(),
                None => {
                    for list_source in self.generic_lists.keys() {
                        info!("Generic lists exist for {list_source:?}");
                    }
                    info!("Current source is {generic_list_token:?}");
                    panic!("No entry in generic list for {name}");
                }
            },
            hir::TypeSpec::Tuple(inner) => {
                let inner = inner
                    .iter()
                    .map(|t| self.type_var_from_hir(loc, t, generic_list_token))
                    .collect();
                TypeVar::tuple(loc, inner)
            }
            hir::TypeSpec::Array { inner, size } => {
                let inner = self.type_var_from_hir(loc, inner, generic_list_token);
                let size = self.hir_type_expr_to_var(size, generic_list_token);

                TypeVar::array(loc, inner, size)
            }
            hir::TypeSpec::Unit(_) => {
                todo!("Support unit type in type inference")
            }
            hir::TypeSpec::Backward(inner) => {
                TypeVar::backward(loc, self.type_var_from_hir(loc, inner, generic_list_token))
            }
            hir::TypeSpec::Wire(inner) => {
                TypeVar::wire(loc, self.type_var_from_hir(loc, inner, generic_list_token))
            }
            hir::TypeSpec::Inverted(inner) => {
                TypeVar::inverted(loc, self.type_var_from_hir(loc, inner, generic_list_token))
            }
            hir::TypeSpec::TraitSelf(_) => {
                panic!("Trying to convert TraitSelf to type inference type var")
            }
        }
    }

    /// Returns the type of the expression with the specified id. Error if no equation
    /// for the specified epxression exists
    pub fn type_of(&self, expr: &TypedExpression) -> Result<TypeVar> {
        for (e, t) in &self.equations {
            if e == expr {
                return Ok(t.clone());
            }
        }
        panic!("Tried looking up the type of {expr:?} but it was not found")
    }

    pub fn new_generic_int(&mut self, loc: Loc<()>, symtab: &SymbolTable) -> TypeVar {
        TypeVar::Known(loc, t_int(symtab), vec![self.new_generic()])
    }

    /// Return a new generic int. The first returned value is int<N>, and the second
    /// value is N
    pub fn new_split_generic_int(
        &mut self,
        loc: Loc<()>,
        symtab: &SymbolTable,
    ) -> (TypeVar, TypeVar) {
        let size = self.new_generic();
        let full = TypeVar::Known(loc, t_int(symtab), vec![size.clone()]);
        (full, size)
    }

    pub fn new_split_generic_uint(
        &mut self,
        loc: Loc<()>,
        symtab: &SymbolTable,
    ) -> (TypeVar, TypeVar) {
        let size = self.new_generic();
        let full = TypeVar::Known(loc, t_uint(symtab), vec![size.clone()]);
        (full, size)
    }

    pub fn new_generic(&mut self) -> TypeVar {
        let id = self.new_typeid();
        TypeVar::Unknown(id, TraitList::empty())
    }

    pub fn new_generic_number(&mut self, ctx: &Context) -> (TypeVar, TypeVar) {
        let number = ctx
            .symtab
            .lookup_trait(&Path::from_strs(&["Number"]).nowhere())
            .expect("Did not find number in symtab")
            .0;
        let id = self.new_typeid();
        let size = self.new_generic();
        let t = TraitReq {
            name: number,
            type_params: vec![size.clone()],
        };
        (TypeVar::Unknown(id, TraitList::from_vec(vec![t])), size)
    }

    pub fn new_generic_with_traits(&mut self, traits: TraitList) -> TypeVar {
        let id = self.new_typeid();
        TypeVar::Unknown(id, traits)
    }

    #[trace_typechecker]
    #[tracing::instrument(level = "trace", skip_all, fields(%entity.name))]
    pub fn visit_entity(&mut self, entity: &Unit, ctx: &Context) -> Result<()> {
        let generic_list = self.create_generic_list(
            GenericListSource::Definition(&entity.name.name_id().inner),
            &entity.head.type_params,
            None,
        )?;

        // Add equations for the inputs
        for (name, t) in &entity.inputs {
            let tvar = self.type_var_from_hir(t.loc(), t, &generic_list);
            self.add_equation(TypedExpression::Name(name.inner.clone()), tvar)
        }

        if entity.head.unit_kind.is_pipeline() {
            self.unify(
                &TypedExpression::Name(entity.inputs[0].0.clone().inner),
                &t_clock(ctx.symtab).at_loc(&entity.head.unit_kind),
                ctx,
            )
            .into_diagnostic(
                entity.inputs[0].1.loc(),
                |diag,
                 Tm {
                     g: got,
                     e: _expected,
                 }| {
                    diag.message(format!(
                        "First pipeline argument must be a clock. Got {got}"
                    ))
                    .primary_label("expected clock")
                },
            )?;
        }

        self.visit_expression(&entity.body, ctx, &generic_list)?;

        // Ensure that the output type matches what the user specified, and unit otherwise
        if let Some(output_type) = &entity.head.output_type {
            let tvar = self.type_var_from_hir(output_type.loc(), output_type, &generic_list);

            self.trace_stack.push(TraceStackEntry::Message(format!(
                "Unifying with output type {tvar:?}"
            )));
            self.unify(&TypedExpression::Id(entity.body.inner.id), &tvar, ctx)
                .into_diagnostic_no_expected_source(
                    &entity.body,
                    |diag,
                     Tm {
                         g: got,
                         e: expected,
                     }| {
                        // FIXME: Might want to check if there is no return value in the block
                        // and in that case suggest adding it.
                        diag.message(format!(
                            "Output type mismatch. Expected {expected}, got {got}"
                        ))
                        .primary_label(format!("Found type {got}"))
                        .secondary_label(output_type, format!("{expected} type specified here"))
                    },
                )?;
        } else {
            // No output type, so unify with the unit type.
            self.unify(
                &TypedExpression::Id(entity.body.inner.id),
                &t_void(ctx.symtab).at_loc(&entity.head.name),
                ctx
            )
            .into_diagnostic_no_expected_source(entity.body.loc(), |diag, Tm{g: got, e: _expected}| {
                diag.message("Output type mismatch")
                    .primary_label(format!("Found type {got}"))
                    .note(format!(
                        "The {} does not specify a return type.\nAdd a return type, or remove the return value.",
                        entity.head.unit_kind.name()
                    ))
            })?;
        }

        self.check_requirements(ctx)?;

        Ok(())
    }

    #[trace_typechecker]
    #[tracing::instrument(level = "trace", skip_all)]
    fn visit_argument_list(
        &mut self,
        args: &Loc<ArgumentList>,
        ctx: &Context,
        generic_list: &GenericListToken,
    ) -> Result<()> {
        for expr in args.expressions() {
            self.visit_expression(expr, ctx, generic_list)?;
        }
        Ok(())
    }

    #[trace_typechecker]
    fn type_check_argument_list(
        &mut self,
        args: &[Argument],
        ctx: &Context,
        generic_list: &GenericListToken,
    ) -> Result<()> {
        for Argument {
            target,
            target_type,
            value,
            kind,
        } in args.iter()
        {
            let target_type = self.type_var_from_hir(value.loc(), target_type, generic_list);

            let loc = match kind {
                hir::param_util::ArgumentKind::Positional => value.loc(),
                hir::param_util::ArgumentKind::Named => value.loc(),
                hir::param_util::ArgumentKind::ShortNamed => target.loc(),
            };

            self.unify(&value.inner, &target_type, ctx)
                .into_diagnostic(
                    loc,
                    |d,
                     Tm {
                         e: expected,
                         g: got,
                     }| {
                        d.message(format!(
                            "Argument type mismatch. Expected {expected} got {got}"
                        ))
                        .primary_label(format!("expected {expected}"))
                    },
                )?;
        }

        Ok(())
    }

    #[tracing::instrument(level = "trace", skip_all)]
    #[trace_typechecker]
    pub fn visit_expression(
        &mut self,
        expression: &Loc<Expression>,
        ctx: &Context,
        generic_list: &GenericListToken,
    ) -> Result<()> {
        let new_type = self.new_generic();
        self.add_equation(TypedExpression::Id(expression.inner.id), new_type);

        // Recurse down the expression
        match &expression.inner.kind {
            ExprKind::Identifier(_) => self.visit_identifier(expression, ctx)?,
            ExprKind::TypeLevelInteger(_) => {
                self.visit_type_level_integer(expression, generic_list, ctx)?
            }
            ExprKind::IntLiteral(_) => self.visit_int_literal(expression, ctx)?,
            ExprKind::BoolLiteral(_) => self.visit_bool_literal(expression, ctx)?,
            ExprKind::BitLiteral(_) => self.visit_bit_literal(expression, ctx)?,
            ExprKind::TupleLiteral(_) => self.visit_tuple_literal(expression, ctx, generic_list)?,
            ExprKind::TupleIndex(_, _) => self.visit_tuple_index(expression, ctx, generic_list)?,
            ExprKind::ArrayLiteral(_) => self.visit_array_literal(expression, ctx, generic_list)?,
            ExprKind::CreatePorts => self.visit_create_ports(expression, ctx, generic_list)?,
            ExprKind::FieldAccess(_, _) => {
                self.visit_field_access(expression, ctx, generic_list)?
            }
            ExprKind::MethodCall { .. } => self.visit_method_call(expression, ctx, generic_list)?,
            ExprKind::Index(_, _) => self.visit_index(expression, ctx, generic_list)?,
            ExprKind::RangeIndex { .. } => self.visit_range_index(expression, ctx, generic_list)?,
            ExprKind::Block(_) => self.visit_block_expr(expression, ctx, generic_list)?,
            ExprKind::If(_, _, _) => self.visit_if(expression, ctx, generic_list)?,
            ExprKind::Match(_, _) => self.visit_match(expression, ctx, generic_list)?,
            ExprKind::BinaryOperator(_, _, _) => {
                self.visit_binary_operator(expression, ctx, generic_list)?
            }
            ExprKind::UnaryOperator(_, _) => {
                self.visit_unary_operator(expression, ctx, generic_list)?
            }
            ExprKind::Call {
                kind: _,
                callee,
                args,
                turbofish,
            } => {
                let head = ctx.symtab.unit_by_id(&callee.inner);
                self.handle_function_like(
                    expression.map_ref(|e| e.id),
                    &expression.get_type(self)?,
                    &callee.inner,
                    &head,
                    args,
                    ctx,
                    true,
                    false,
                    turbofish.as_ref().map(|turbofish| TurbofishCtx {
                        turbofish,
                        prev_generic_list: generic_list,
                        type_ctx: ctx,
                    }),
                )?;
            }
            ExprKind::PipelineRef { .. } => {
                self.visit_pipeline_ref(expression, ctx)?;
            }
            ExprKind::StageReady | ExprKind::StageValid => {
                self.unify_expression_generic_error(
                    expression,
                    &t_bool(ctx.symtab).at_loc(expression),
                    ctx,
                )?;
            }
            ExprKind::Null => {}
        }
        Ok(())
    }

    // Common handler for entities, functions and pipelines
    #[tracing::instrument(level = "trace", skip_all, fields(%name))]
    #[trace_typechecker]
    fn handle_function_like(
        &mut self,
        expression_id: Loc<u64>,
        expression_type: &TypeVar,
        name: &NameID,
        head: &Loc<UnitHead>,
        args: &Loc<ArgumentList>,
        ctx: &Context,
        // Whether or not to visit the argument expressions passed to the function here. This
        // should not be done if the expressoins have been visited before, for example, when
        // handling methods
        visit_args: bool,
        // If we are calling a method, we have an implicit self argument which means
        // that any error reporting number of arguments should be reduced by one
        is_method: bool,
        turbofish: Option<TurbofishCtx>,
    ) -> Result<()> {
        // Add new symbols for all the type parameters
        let generic_list = self.create_generic_list(
            GenericListSource::Expression(expression_id.inner),
            &head.type_params,
            turbofish,
        )?;

        if visit_args {
            self.visit_argument_list(args, ctx, &generic_list)?;
        }

        let type_params = &head.type_params;

        // Special handling of built in functions
        macro_rules! handle_special_functions {
            ($([$($path:expr),*] => $handler:expr),*) => {
                $(
                    let path = Path(vec![$(Identifier($path.to_string()).nowhere()),*]).nowhere();
                    if ctx.symtab
                        .try_lookup_final_id(&path)
                        .map(|n| &n == name)
                        .unwrap_or(false)
                    {
                        $handler
                    };
                )*
            }
        }

        // NOTE: These would be better as a closure, but unfortunately that does not satisfy
        // the borrow checker
        macro_rules! generic_arg {
            ($idx:expr) => {
                self.get_generic_list(&generic_list)[&type_params[$idx].name_id()].clone()
            };
        }

        let matched_args = match_args_with_params(args, &head.inputs, is_method).map_err(|e| {
            let diag: Diagnostic = e.into();
            diag.secondary_label(
                head,
                format!("{kind} defined here", kind = head.unit_kind.name()),
            )
        })?;

        handle_special_functions! {
            ["std", "conv", "concat"] => {
                self.handle_concat(
                    expression_id,
                    generic_arg!(0),
                    generic_arg!(1),
                    generic_arg!(2),
                    &matched_args,
                    ctx
                )?
            },
            ["std", "conv", "trunc"] => {
                self.handle_trunc(
                    expression_id,
                    generic_arg!(0),
                    generic_arg!(1),
                    &matched_args,
                    ctx
                )?
            },
            ["std", "mem", "clocked_memory"]  => {
                let num_elements = generic_arg!(0);
                let addr_size = generic_arg!(2);

                self.handle_clocked_memory(num_elements, addr_size, &matched_args, ctx)?
            },
            // NOTE: the last argument of _init does not need special treatment, so
            // we can use the same code path for both for now
            ["std", "mem", "clocked_memory_init"]  => {
                let num_elements = generic_arg!(0);
                let addr_size = generic_arg!(2);

                self.handle_clocked_memory(num_elements, addr_size, &matched_args, ctx)?
            },
            ["std", "mem", "read_memory"]  => {
                let addr_size = generic_arg!(0);
                let num_elements = generic_arg!(2);

                self.handle_read_memory(num_elements, addr_size, &matched_args, ctx)?
            }
        };

        // Unify the types of the arguments
        self.type_check_argument_list(&matched_args, ctx, &generic_list)?;

        let return_type = head
            .output_type
            .as_ref()
            .map(|o| self.type_var_from_hir(expression_id.loc(), o, &generic_list))
            .unwrap_or_else(|| TypeVar::Known(expression_id.loc(), t_void(ctx.symtab), vec![]));

        self.unify(expression_type, &return_type, ctx)
            .into_default_diagnostic(expression_id.loc())?;

        Ok(())
    }

    pub fn handle_concat(
        &mut self,
        expression_id: Loc<u64>,
        source_lhs_ty: TypeVar,
        source_rhs_ty: TypeVar,
        source_result_ty: TypeVar,
        args: &[Argument],
        ctx: &Context,
    ) -> Result<()> {
        let (lhs_type, lhs_size) = self.new_generic_number(ctx);
        let (rhs_type, rhs_size) = self.new_generic_number(ctx);
        let (result_type, result_size) = self.new_generic_number(ctx);
        self.unify(&source_lhs_ty, &lhs_type, ctx)
            .into_default_diagnostic(args[0].value.loc())?;
        self.unify(&source_rhs_ty, &rhs_type, ctx)
            .into_default_diagnostic(args[1].value.loc())?;
        self.unify(&source_result_ty, &result_type, ctx)
            .into_default_diagnostic(expression_id.loc())?;

        // Result size is sum of input sizes
        self.add_constraint(
            result_size.clone(),
            ce_var(&lhs_size) + ce_var(&rhs_size),
            expression_id.loc(),
            &result_size,
            ConstraintSource::Concatenation,
        );
        self.add_constraint(
            lhs_size.clone(),
            ce_var(&result_size) + -ce_var(&rhs_size),
            args[0].value.loc(),
            &lhs_size,
            ConstraintSource::Concatenation,
        );
        self.add_constraint(
            rhs_size.clone(),
            ce_var(&result_size) + -ce_var(&lhs_size),
            args[1].value.loc(),
            &rhs_size,
            ConstraintSource::Concatenation,
        );

        self.add_requirement(Requirement::SharedBase(vec![
            lhs_type.at_loc(&args[0].value),
            rhs_type.at_loc(&args[1].value),
            result_type.at_loc(&expression_id.loc()),
        ]));
        Ok(())
    }

    pub fn handle_trunc(
        &mut self,
        expression_id: Loc<u64>,
        source_in_ty: TypeVar,
        source_result_ty: TypeVar,
        args: &[Argument],
        ctx: &Context,
    ) -> Result<()> {
        let (in_ty, _) = self.new_generic_number(ctx);
        let (result_type, _) = self.new_generic_number(ctx);
        self.unify(&source_in_ty, &in_ty, ctx)
            .into_default_diagnostic(args[0].value.loc())?;
        self.unify(&source_result_ty, &result_type, ctx)
            .into_default_diagnostic(expression_id.loc())?;

        self.add_requirement(Requirement::SharedBase(vec![
            in_ty.at_loc(&args[0].value),
            result_type.at_loc(&expression_id.loc()),
        ]));
        Ok(())
    }

    pub fn handle_clocked_memory(
        &mut self,
        num_elements: TypeVar,
        addr_size_arg: TypeVar,
        args: &[Argument],
        ctx: &Context,
    ) -> Result<()> {
        let (addr_type, addr_size) = self.new_split_generic_uint(args[1].value.loc(), ctx.symtab);
        let port_type = TypeVar::array(
            args[1].value.loc(),
            TypeVar::tuple(
                args[1].value.loc(),
                vec![self.new_generic(), addr_type, self.new_generic()],
            ),
            self.new_generic(),
        );

        self.add_constraint(
            addr_size.clone(),
            bits_to_store(ce_var(&num_elements) - ce_int(1.to_bigint())),
            args[1].value.loc(),
            &port_type,
            ConstraintSource::MemoryIndexing,
        );

        // NOTE: Unwrap is safe, size is still generic at this point
        self.unify(&addr_size, &addr_size_arg, ctx).unwrap();
        self.unify_expression_generic_error(args[1].value, &port_type, ctx)?;

        Ok(())
    }

    pub fn handle_read_memory(
        &mut self,
        num_elements: TypeVar,
        addr_size_arg: TypeVar,
        args: &[Argument],
        ctx: &Context,
    ) -> Result<()> {
        let (addr_type, addr_size) = self.new_split_generic_uint(args[1].value.loc(), ctx.symtab);

        self.add_constraint(
            addr_size.clone(),
            bits_to_store(ce_var(&num_elements) - ce_int(1.to_bigint())),
            args[1].value.loc(),
            &addr_type,
            ConstraintSource::MemoryIndexing,
        );

        // NOTE: Unwrap is safe, size is still generic at this point
        self.unify(&addr_size, &addr_size_arg, ctx).unwrap();

        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self, turbofish))]
    pub fn create_generic_list(
        &mut self,
        source: GenericListSource,
        params: &[Loc<TypeParam>],
        turbofish: Option<TurbofishCtx>,
    ) -> Result<GenericListToken> {
        let turbofish_params = if let Some(turbofish) = turbofish.as_ref() {
            if params.is_empty() {
                return Err(Diagnostic::error(
                    turbofish.turbofish,
                    "Turbofish on non-generic function",
                )
                .primary_label("Turbofish on non-generic function")
                .into());
            }
            if turbofish.turbofish.len() != params.len() {
                return Err(Diagnostic::error(
                    turbofish.turbofish,
                    "Wrong number of type parameters",
                )
                .primary_label(format!(
                    "Expected {} type parameter{s}",
                    params.len(),
                    s = if params.len() != 1 { "s" } else { "" }
                ))
                .secondary_label(
                    ().between_locs(&params[0], &params[1]),
                    format!(
                        "Because this has {} parameter{s}",
                        params.len(),
                        s = if params.len() != 1 { "s" } else { "" }
                    ),
                ));
            }

            turbofish
                .turbofish
                .inner
                .iter()
                .map(|p| Some(p.clone()))
                .collect()
        } else {
            params.iter().map(|_| None).collect::<Vec<_>>()
        };

        let new_list = params
            .iter()
            .enumerate()
            .map(|(i, param)| {
                let name = match &param.inner {
                    hir::TypeParam::TypeName(_, name) => name.clone(),
                    hir::TypeParam::Integer(_, name) => name.clone(),
                };

                let t = self.new_generic();

                if let Some(tf) = &turbofish_params[i] {
                    let tf_ctx = turbofish.as_ref().unwrap();
                    let ty = self.hir_type_expr_to_var(tf, tf_ctx.prev_generic_list);
                    self.unify(&ty, &t, tf_ctx.type_ctx)
                        .into_default_diagnostic(param)?;
                }
                Ok((name, self.check_var_for_replacement(t)))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .map(|(name, t)| (name, t.clone()))
            .collect::<HashMap<_, _>>();

        self.trace_stack
            .push(TraceStackEntry::NewGenericList(new_list.clone()));

        Ok(self.add_mapped_generic_list(source, new_list))
    }

    /// Adds a generic list with parameters already mapped to types
    pub fn add_mapped_generic_list(
        &mut self,
        source: GenericListSource,
        mapping: HashMap<NameID, TypeVar>,
    ) -> GenericListToken {
        let reference = match source {
            GenericListSource::Anonymous => GenericListToken::Anonymous(self.generic_lists.len()),
            GenericListSource::Definition(name) => GenericListToken::Definition(name.clone()),
            GenericListSource::Expression(id) => GenericListToken::Expression(id),
        };

        if self
            .generic_lists
            .insert(reference.clone(), mapping)
            .is_some()
        {
            panic!("A generic list already existed for {reference:?}");
        }
        reference
    }

    #[tracing::instrument(level = "trace", skip_all)]
    #[trace_typechecker]
    pub fn visit_block(
        &mut self,
        block: &Block,
        ctx: &Context,
        generic_list: &GenericListToken,
    ) -> Result<()> {
        for statement in &block.statements {
            self.visit_statement(statement, ctx, generic_list)?;
        }
        if let Some(result) = &block.result {
            self.visit_expression(result, ctx, generic_list)?;
        }
        Ok(())
    }

    #[trace_typechecker]
    pub fn visit_pattern(
        &mut self,
        pattern: &Loc<Pattern>,
        ctx: &Context,
        generic_list: &GenericListToken,
    ) -> Result<()> {
        let new_type = self.new_generic();
        self.add_equation(TypedExpression::Id(pattern.inner.id), new_type);
        match &pattern.inner.kind {
            hir::PatternKind::Integer(_) => {
                let (num_t, _) = &self.new_generic_number(ctx);
                self.unify(pattern, num_t, ctx)
                    .expect("Failed to unify new_generic with int");
            }
            hir::PatternKind::Bool(_) => {
                self.unify(pattern, &t_bool(ctx.symtab).at_loc(pattern), ctx)
                    .expect("Expected new_generic with boolean");
            }
            hir::PatternKind::Name { name, pre_declared } => {
                if !pre_declared {
                    self.add_equation(
                        TypedExpression::Name(name.clone().inner),
                        pattern.get_type(self)?,
                    );
                }
                self.unify(
                    &TypedExpression::Id(pattern.id),
                    &TypedExpression::Name(name.clone().inner),
                    ctx,
                )
                .into_default_diagnostic(name.loc())?;
            }
            hir::PatternKind::Tuple(subpatterns) => {
                for pattern in subpatterns {
                    self.visit_pattern(pattern, ctx, generic_list)?;
                }
                let tuple_type = TypeVar::tuple(
                    pattern.loc(),
                    subpatterns
                        .iter()
                        .map(|pattern| {
                            let p_type = pattern.get_type(self)?;
                            Ok(p_type)
                        })
                        .collect::<Result<_>>()?,
                );

                self.unify(pattern, &tuple_type, ctx)
                    .expect("Unification of new_generic with tuple type cannot fail");
            }
            hir::PatternKind::Type(name, args) => {
                let (condition_type, params, generic_list) =
                    match ctx.symtab.patternable_type_by_id(name).inner {
                        Patternable {
                            kind: PatternableKind::Enum,
                            params: _,
                        } => {
                            let enum_variant = ctx.symtab.enum_variant_by_id(name).inner;
                            let generic_list = self.create_generic_list(
                                GenericListSource::Anonymous,
                                &enum_variant.type_params,
                                None,
                            )?;

                            let condition_type = self.type_var_from_hir(
                                pattern.loc(),
                                &enum_variant.output_type,
                                &generic_list,
                            );

                            (condition_type, enum_variant.params, generic_list)
                        }
                        Patternable {
                            kind: PatternableKind::Struct,
                            params: _,
                        } => {
                            let s = ctx.symtab.struct_by_id(name).inner;
                            let generic_list = self.create_generic_list(
                                GenericListSource::Anonymous,
                                &s.type_params,
                                None,
                            )?;

                            let condition_type =
                                self.type_var_from_hir(pattern.loc(), &s.self_type, &generic_list);

                            (condition_type, s.params, generic_list)
                        }
                    };

                self.unify(pattern, &condition_type, ctx)
                    .expect("Unification of new_generic with enum cannot fail");

                for (
                    PatternArgument {
                        target,
                        value: pattern,
                        kind,
                    },
                    Parameter {
                        name: _,
                        ty: target_type,
                        no_mangle: _,
                    },
                ) in args.iter().zip(params.0.iter())
                {
                    self.visit_pattern(pattern, ctx, &generic_list)?;
                    let target_type =
                        self.type_var_from_hir(target_type.loc(), target_type, &generic_list);

                    let loc = match kind {
                        hir::ArgumentKind::Positional => pattern.loc(),
                        hir::ArgumentKind::Named => pattern.loc(),
                        hir::ArgumentKind::ShortNamed => target.loc(),
                    };

                    self.unify(pattern, &target_type, ctx).into_diagnostic(
                        loc,
                        |d,
                         Tm {
                             e: expected,
                             g: got,
                         }| {
                            d.message(format!(
                                "Argument type mismatch. Expected {expected} got {got}"
                            ))
                            .primary_label(format!("expected {expected}"))
                        },
                    )?;
                }
            }
        }
        Ok(())
    }

    #[trace_typechecker]
    pub fn visit_wal_trace(
        &mut self,
        trace: &Loc<WalTrace>,
        ctx: &Context,
        generic_list: &GenericListToken,
    ) -> Result<()> {
        let WalTrace { clk, rst } = &trace.inner;
        clk.as_ref()
            .map(|x| {
                self.visit_expression(x, ctx, generic_list)?;
                self.unify_expression_generic_error(x, &t_clock(ctx.symtab).at_loc(trace), ctx)
            })
            .transpose()?;
        rst.as_ref()
            .map(|x| {
                self.visit_expression(x, ctx, generic_list)?;
                self.unify_expression_generic_error(x, &t_bool(ctx.symtab).at_loc(trace), ctx)
            })
            .transpose()?;
        Ok(())
    }

    #[trace_typechecker]
    pub fn visit_statement(
        &mut self,
        stmt: &Loc<Statement>,
        ctx: &Context,
        generic_list: &GenericListToken,
    ) -> Result<()> {
        match &stmt.inner {
            Statement::Binding(Binding {
                pattern,
                ty,
                value,
                wal_trace,
            }) => {
                trace!("Visiting `let {} = ..`", pattern.kind);
                self.visit_expression(value, ctx, generic_list)?;

                self.visit_pattern(pattern, ctx, generic_list)?;

                self.unify(&TypedExpression::Id(pattern.id), value, ctx)
                    .into_diagnostic(
                        pattern.loc(),
                        error_pattern_type_mismatch(
                            ty.as_ref().map(|t| t.loc()).unwrap_or_else(|| value.loc()),
                        ),
                    )?;

                if let Some(t) = ty {
                    let tvar = self.type_var_from_hir(t.loc(), t, generic_list);
                    self.unify(&TypedExpression::Id(pattern.id), &tvar, ctx)
                        .into_default_diagnostic(value.loc())?;
                }

                wal_trace
                    .as_ref()
                    .map(|wt| self.visit_wal_trace(wt, ctx, generic_list))
                    .transpose()?;

                Ok(())
            }
            Statement::Register(reg) => self.visit_register(reg, ctx, generic_list),
            Statement::Declaration(names) => {
                for name in names {
                    let new_type = self.new_generic();
                    self.add_equation(TypedExpression::Name(name.clone().inner), new_type);
                }
                Ok(())
            }
            Statement::PipelineRegMarker(cond) => {
                if let Some(cond) = cond {
                    self.visit_expression(cond, ctx, generic_list)?;
                    self.unify_expression_generic_error(
                        cond,
                        &t_bool(ctx.symtab).at_loc(cond),
                        ctx,
                    )?;
                }
                Ok(())
            }
            // This statement have no effect on the types
            Statement::Label(_) => Ok(()),
            Statement::WalSuffixed { .. } => Ok(()),
            Statement::Assert(expr) => {
                self.visit_expression(expr, ctx, generic_list)?;
                self.unify_expression_generic_error(expr, &t_bool(ctx.symtab).at_loc(stmt), ctx)?;
                Ok(())
            }
            Statement::Set { target, value } => {
                self.visit_expression(target, ctx, generic_list)?;
                self.visit_expression(value, ctx, generic_list)?;

                let inner_type = self.new_generic();
                let outer_type = TypeVar::backward(stmt.loc(), inner_type.clone());
                self.unify_expression_generic_error(target, &outer_type, ctx)?;
                self.unify_expression_generic_error(value, &inner_type, ctx)?;

                Ok(())
            }
        }
    }

    #[trace_typechecker]
    pub fn visit_register(
        &mut self,
        reg: &Register,
        ctx: &Context,
        generic_list: &GenericListToken,
    ) -> Result<()> {
        self.visit_pattern(&reg.pattern, ctx, generic_list)?;

        let type_spec_type = &reg
            .value_type
            .as_ref()
            .map(|t| self.type_var_from_hir(t.loc(), t, generic_list).at_loc(t));

        // We need to do this before visiting value, in case it constrains the
        // type of the identifiers in the pattern
        if let Some(tvar) = type_spec_type {
            self.unify(&TypedExpression::Id(reg.pattern.id), tvar, ctx)
                .into_diagnostic_no_expected_source(
                    reg.pattern.loc(),
                    error_pattern_type_mismatch(tvar.loc()),
                )?;
        }

        self.visit_expression(&reg.clock, ctx, generic_list)?;
        self.visit_expression(&reg.value, ctx, generic_list)?;

        if let Some(tvar) = type_spec_type {
            self.unify(&reg.value, tvar, ctx)
                .into_default_diagnostic(reg.value.loc())?;
        }

        if let Some((rst_cond, rst_value)) = &reg.reset {
            self.visit_expression(rst_cond, ctx, generic_list)?;
            self.visit_expression(rst_value, ctx, generic_list)?;
            // Ensure cond is a boolean
            self.unify(&rst_cond.inner, &t_bool(ctx.symtab).at_loc(&rst_cond), ctx)
                .into_diagnostic(
                    rst_cond.loc(),
                    |diag,
                     Tm {
                         g: got,
                         e: _expected,
                     }| {
                        diag.message(format!(
                            "Register reset condition must be a bool, got {got}"
                        ))
                        .primary_label("expected bool")
                    },
                )?;

            // Ensure the reset value has the same type as the register itself
            self.unify(&rst_value.inner, &reg.value.inner, ctx)
                .into_diagnostic(
                    rst_value.loc(),
                    |diag,
                     Tm {
                         g: got,
                         e: expected,
                     }| {
                        diag.message(format!(
                            "Register reset value mismatch. Expected {expected} got {got}"
                        ))
                        .primary_label(format!("expected {expected}"))
                        .secondary_label(&reg.pattern, format!("because this has type {expected}"))
                    },
                )?;
        }

        if let Some(initial) = &reg.initial {
            self.visit_expression(initial, ctx, generic_list)?;

            self.unify(&initial.inner, &reg.value.inner, ctx)
                .into_diagnostic(
                    initial.loc(),
                    |diag,
                     Tm {
                         g: got,
                         e: expected,
                     }| {
                        diag.message(format!(
                            "Register initial value mismatch. Expected {expected} got {got}"
                        ))
                        .primary_label(format!("expected {expected}, got {got}"))
                        .secondary_label(&reg.pattern, format!("because this has type {got}"))
                    },
                )?;
        }

        self.unify(&reg.clock, &t_clock(ctx.symtab).at_loc(&reg.clock), ctx)
            .into_diagnostic(
                reg.clock.loc(),
                |diag,
                 Tm {
                     g: got,
                     e: _expected,
                 }| {
                    diag.message(format!("Expected clock, got {got}"))
                        .primary_label("expected clock")
                },
            )?;

        self.unify(&TypedExpression::Id(reg.pattern.id), &reg.value, ctx)
            .into_diagnostic(
                reg.pattern.loc(),
                error_pattern_type_mismatch(reg.value.loc()),
            )?;

        Ok(())
    }
}

// Private helper functions
impl TypeState {
    fn new_typeid(&mut self) -> u64 {
        let result = self.next_typeid;
        self.next_typeid += 1;
        result
    }

    fn check_var_for_replacement(&self, var: TypeVar) -> TypeVar {
        if let Some(new) = self.replacements.get(&var) {
            return new.clone();
        };
        match var {
            TypeVar::Known(loc, base, params) => TypeVar::Known(
                loc,
                base,
                params
                    .into_iter()
                    .map(|p| self.check_var_for_replacement(p))
                    .collect(),
            ),
            TypeVar::Unknown(id, traits) => TypeVar::Unknown(
                id,
                TraitList::from_vec(
                    traits
                        .inner
                        .iter()
                        .map(|t| TraitReq {
                            name: t.name.clone(),
                            type_params: t
                                .type_params
                                .iter()
                                .map(|v| self.check_var_for_replacement(v.clone()))
                                .collect(),
                        })
                        .collect(),
                ),
            ),
        }
    }

    fn check_expr_for_replacement(&self, val: ConstraintExpr) -> ConstraintExpr {
        match val {
            v @ ConstraintExpr::Integer(_) => v,
            ConstraintExpr::Var(var) => ConstraintExpr::Var(self.check_var_for_replacement(var)),
            ConstraintExpr::Sum(lhs, rhs) => ConstraintExpr::Sum(
                Box::new(self.check_expr_for_replacement(*lhs)),
                Box::new(self.check_expr_for_replacement(*rhs)),
            ),
            ConstraintExpr::Sub(inner) => {
                ConstraintExpr::Sub(Box::new(self.check_expr_for_replacement(*inner)))
            }
            ConstraintExpr::BitsToRepresent(inner) => {
                ConstraintExpr::BitsToRepresent(Box::new(self.check_expr_for_replacement(*inner)))
            }
        }
    }

    pub fn add_equation(&mut self, expression: TypedExpression, var: TypeVar) {
        let var = self.check_var_for_replacement(var);

        self.trace_stack.push(TraceStackEntry::AddingEquation(
            expression.clone(),
            var.clone(),
        ));
        if let Some(prev) = self.equations.insert(expression.clone(), var.clone()) {
            let var = var.clone();
            let expr = expression.clone();
            println!("{}", format_trace_stack(&self));
            panic!("Adding equation for {} == {} but a previous eq exists.\n\tIt was previously bound to {}", expr, var, prev)
        }
    }

    fn add_constraint(
        &mut self,
        lhs: TypeVar,
        rhs: ConstraintExpr,
        loc: Loc<()>,
        inside: &TypeVar,
        source: ConstraintSource,
    ) {
        let replaces = lhs.clone();
        let lhs = self.check_var_for_replacement(lhs);
        let rhs = self
            .check_expr_for_replacement(rhs)
            .with_context(&replaces, &inside.clone(), source)
            .at_loc(&loc);

        self.constraints.add_constraint(lhs, rhs);
    }

    fn add_requirement(&mut self, requirement: Requirement) {
        macro_rules! replace {
            ($name:expr) => {
                self.check_var_for_replacement($name.inner.clone())
                    .at_loc(&$name)
            };
        }

        let replaced = match requirement {
            Requirement::HasField {
                target_type,
                field,
                expr,
            } => Requirement::HasField {
                field,
                target_type: replace!(target_type),
                expr: replace!(expr),
            },
            Requirement::HasMethod {
                expr_id,
                target_type,
                method,
                expr,
                args,
            } => Requirement::HasMethod {
                expr_id,
                target_type: replace!(target_type),
                method,
                expr: replace!(expr),
                args,
            },
            Requirement::FitsIntLiteral { value, target_type } => Requirement::FitsIntLiteral {
                value: match value {
                    ConstantInt::Generic(var) => {
                        ConstantInt::Generic(replace!(var.clone().nowhere()).inner)
                    }
                    lit @ ConstantInt::Literal(_) => lit,
                },
                target_type: replace!(target_type),
            },
            Requirement::SharedBase(types) => {
                Requirement::SharedBase(types.iter().map(|ty| replace!(ty)).collect())
            }
        };

        self.trace_stack
            .push(TraceStackEntry::AddRequirement(replaced.clone()));
        self.requirements.push(replaced)
    }

    #[cfg(test)]
    fn add_eq_from_tvar(&mut self, expression: TypedExpression, var: TypeVar) {
        self.add_equation(expression, var)
    }

    /// Performs unification but does not update constraints. This is done to avoid
    /// updating constraints more often than necessary. Technically, constraints can
    /// be updated even less often, but `unify` is a pretty natural point to do so.

    fn unify_inner(
        &mut self,
        e1: &impl HasType,
        e2: &impl HasType,
        ctx: &Context,
    ) -> std::result::Result<TypeVar, UnificationError> {
        let v1 = e1
            .get_type(self)
            .expect("Tried to unify types but the lhs was not found");
        let v2 = e2
            .get_type(self)
            .expect("Tried to unify types but the rhs was not found");

        trace!("Unifying {v1} with {v2}");

        let v1 = self.check_var_for_replacement(v1);
        let v2 = self.check_var_for_replacement(v2);

        self.trace_stack
            .push(TraceStackEntry::TryingUnify(v1.clone(), v2.clone()));

        // Copy the original types so we can properly trace the progress of the
        // unification
        let v1cpy = v1.clone();
        let v2cpy = v2.clone();

        macro_rules! err_producer {
            () => {{
                self.trace_stack
                    .push(TraceStackEntry::Message("Produced error".to_string()));
                UnificationError::Normal(Tm {
                    g: UnificationTrace::new(self.check_var_for_replacement(v1)),
                    e: UnificationTrace::new(self.check_var_for_replacement(v2)),
                })
            }};
        }

        macro_rules! unify_if {
            ($condition:expr, $new_type:expr, $replaced_type:expr) => {
                if $condition {
                    Ok(($new_type, $replaced_type))
                } else {
                    Err(err_producer!())
                }
            };
        }

        macro_rules! try_with_context {
            ($value: expr) => {
                try_with_context!($value, v1, v2)
            };
            ($value: expr, $v1:expr, $v2:expr) => {
                match $value {
                    Ok(result) => result,
                    e => {
                        self.trace_stack
                            .push(TraceStackEntry::Message("Adding context".to_string()));
                        return e.add_context($v1.clone(), $v2.clone());
                    }
                }
            };
        }

        macro_rules! unify_params_ {
            ($p1:expr, $p2:expr) => {
                if $p1.len() != $p2.len() {
                    return Err(err_producer!());
                }

                for (t1, t2) in $p1.iter().zip($p2.iter()) {
                    try_with_context!(self.unify_inner(t1, t2, ctx));
                }
            };
        }

        // Figure out the most general type, and take note if we need to
        // do any replacement of the types in the rest of the state
        let result = match (&v1, &v2) {
            (TypeVar::Known(_, t1, p1), TypeVar::Known(_, t2, p2)) => {
                macro_rules! unify_params {
                    () => {
                        unify_params_!(p1, p2)
                    };
                }
                match (t1, t2) {
                    (KnownType::Integer(val1), KnownType::Integer(val2)) => {
                        unify_if!(val1 == val2, v1, vec![])
                    }
                    (KnownType::Named(n1), KnownType::Named(n2)) => {
                        match (
                            &ctx.symtab.type_symbol_by_id(&n1).inner,
                            &ctx.symtab.type_symbol_by_id(&n2).inner,
                        ) {
                            (TypeSymbol::Declared(_, _), TypeSymbol::Declared(_, _)) => {
                                if n1 != n2 {
                                    return Err(err_producer!());
                                }

                                unify_params!();
                                let new_ts1 = ctx.symtab.type_symbol_by_id(&n1).inner;
                                let new_ts2 = ctx.symtab.type_symbol_by_id(&n2).inner;
                                let new_v1 = e1
                                    .get_type(self)
                                    .expect("Tried to unify types but the lhs was not found");
                                unify_if!(
                                    new_ts1 == new_ts2,
                                    self.check_var_for_replacement(new_v1),
                                    vec![]
                                )
                            }
                            (TypeSymbol::Declared(_, _), TypeSymbol::GenericArg { traits }) => {
                                if !traits.is_empty() {
                                    todo!("Implement trait unifictaion");
                                }
                                Ok((v1, vec![v2]))
                            }
                            (TypeSymbol::GenericArg { traits }, TypeSymbol::Declared(_, _)) => {
                                if !traits.is_empty() {
                                    todo!("Implement trait unifictaion");
                                }
                                Ok((v2, vec![v1]))
                            }
                            (
                                TypeSymbol::GenericArg { traits: ltraits },
                                TypeSymbol::GenericArg { traits: rtraits },
                            ) => {
                                if !ltraits.is_empty() || !rtraits.is_empty() {
                                    todo!("Implement trait unifictaion");
                                }
                                Ok((v1, vec![v2]))
                            }
                            (TypeSymbol::Declared(_, _), TypeSymbol::GenericInt) => todo!(),
                            (TypeSymbol::GenericArg { traits: _ }, TypeSymbol::GenericInt) => {
                                todo!()
                            }
                            (TypeSymbol::GenericInt, TypeSymbol::Declared(_, _)) => todo!(),
                            (TypeSymbol::GenericInt, TypeSymbol::GenericArg { traits: _ }) => {
                                todo!()
                            }
                            (TypeSymbol::GenericInt, TypeSymbol::GenericInt) => todo!(),
                        }
                    }
                    (KnownType::Array, KnownType::Array)
                    | (KnownType::Tuple, KnownType::Tuple)
                    | (KnownType::Wire, KnownType::Wire)
                    | (KnownType::Backward, KnownType::Backward)
                    | (KnownType::Inverted, KnownType::Inverted) => {
                        unify_params!();
                        // Note, replacements should only occur when a variable goes from Unknown
                        // to Known, not when the base is the same. Replacements take care
                        // of parameters. Therefore, None is returned here
                        Ok((self.check_var_for_replacement(v2), vec![]))
                    }
                    (_, _) => Err(err_producer!()),
                }
            }
            // Unknown with unknown requires merging traits
            (TypeVar::Unknown(_, traits1), TypeVar::Unknown(_, traits2)) => {
                let new_trait_names = traits1
                    .inner
                    .iter()
                    .chain(traits2.inner.iter())
                    .cloned()
                    .map(|t| t.name)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();

                let new_traits = new_trait_names
                    .iter()
                    .map(
                        |name| match (traits1.get_trait(&name), traits2.get_trait(&name)) {
                            (Some(req1), Some(req2)) => {
                                let new_params = req1
                                    .type_params
                                    .iter()
                                    .zip(req2.type_params.iter())
                                    .map(|(p1, p2)| self.unify(p1, p2, ctx))
                                    .collect::<std::result::Result<_, UnificationError>>()?;

                                Ok(TraitReq {
                                    name: name.clone(),
                                    type_params: new_params,
                                })
                            }
                            (Some(t), None) => Ok(t.clone()),
                            (None, Some(t)) => Ok(t.clone()),
                            (None, None) => panic!("Found a trait but neither side has it"),
                        },
                    )
                    .collect::<std::result::Result<Vec<_>, UnificationError>>()?;

                let new_t = self.new_generic_with_traits(TraitList::from_vec(new_traits));

                Ok((new_t, vec![v1, v2]))
            }
            (other @ TypeVar::Known(loc, base, params), uk @ TypeVar::Unknown(_, traits))
            | (uk @ TypeVar::Unknown(_, traits), other @ TypeVar::Known(loc, base, params)) => {
                let trait_is_expected = match (&v1, &v2) {
                    (TypeVar::Known(_, _, _), _) => true,
                    _ => false,
                };

                self.ensure_impls(other, traits, trait_is_expected, ctx)?;

                let mut new_params = params.clone();
                for t in &traits.inner {
                    if !t.type_params.is_empty() {
                        if t.type_params.len() != params.len() {
                            return Err(err_producer!());
                        }

                        // If we don't cheat a bit, we'll get bad error messages in this case when
                        // we unify, for example, `Number<10>` with `uint<9>`. Since we know the
                        // outer types match already, we'll create a fake type for the lhs where
                        // we preemptively crate uint<T>
                        let fake_type =
                            TypeVar::Known(loc.clone(), base.clone(), t.type_params.clone());

                        new_params = t
                            .type_params
                            .iter()
                            .zip(new_params.iter())
                            .map(|(t1, t2)| {
                                Ok(try_with_context!(
                                    self.unify_inner(t1, t2, ctx),
                                    fake_type,
                                    other
                                ))
                            })
                            .collect::<std::result::Result<_, _>>()?;
                    }
                }

                let new = TypeVar::Known(loc.clone(), base.clone(), new_params);

                Ok((new, vec![uk.clone()]))
            }
        };

        let (new_type, replaced_types) = result?;

        self.trace_stack.push(TraceStackEntry::Unified(
            v1cpy,
            v2cpy,
            new_type.clone(),
            replaced_types.clone(),
        ));

        for replaced_type in replaced_types {
            self.replacements
                .insert(replaced_type.clone(), new_type.clone());

            for rhs in self.equations.values_mut() {
                TypeState::replace_type_var(rhs, &replaced_type, &new_type)
            }
            for generic_list in &mut self.generic_lists.values_mut() {
                for (_, lhs) in generic_list.iter_mut() {
                    TypeState::replace_type_var(lhs, &replaced_type, &new_type)
                }
            }
            for requirement in &mut self.requirements {
                requirement.replace_type_var(&replaced_type, &new_type)
            }

            self.constraints.inner = self
                .constraints
                .inner
                .clone()
                .into_iter()
                .map(|(mut lhs, mut rhs)| {
                    TypeState::replace_type_var(&mut lhs, &replaced_type, &new_type);
                    TypeState::replace_type_var_in_constraint_rhs(
                        &mut rhs,
                        &replaced_type,
                        &new_type,
                    );

                    (lhs, rhs)
                })
                .collect()
        }

        Ok(new_type)
    }

    #[tracing::instrument(level = "trace", skip_all)]
    pub fn unify(
        &mut self,
        e1: &impl HasType,
        e2: &impl HasType,
        ctx: &Context,
    ) -> std::result::Result<TypeVar, UnificationError> {
        let new_type = self.unify_inner(e1, e2, ctx)?;

        // With replacement done, some of our constraints may have been updated to provide
        // more type inference information. Try to do unification of those new constraints too
        loop {
            trace!("Updating constraints");
            let new_info = self.constraints.update_constraints();

            if new_info.is_empty() {
                break;
            }

            for constraint in new_info {
                trace!(
                    "Constraint replaces {} with {:?}",
                    constraint.inner.0,
                    constraint.inner.1
                );

                let ((var, replacement), loc) = constraint.split_loc();

                self.trace_stack
                    .push(TraceStackEntry::InferringFromConstraints(
                        var.clone(),
                        replacement.val.clone(),
                    ));

                let var = self.check_var_for_replacement(var);

                if replacement.val < BigInt::zero() {
                    // lifeguard spade#126
                    return Err(Diagnostic::bug(loc, "Inferred integer < 0")
                        .primary_label(format!(
                            "{} is not >= 0 in {}",
                            replacement.val, replacement.context.inside
                        ))
                        .into());
                }

                // NOTE: safe unwrap. We already checked the constraint above
                let expected_type = &KnownType::Integer(replacement.val.to_biguint().unwrap());
                match self.unify_inner(&expected_type.clone().at_loc(&loc), &var, ctx) {
                    Ok(_) => {}
                    Err(UnificationError::Normal(Tm { mut e, mut g })) => {
                        let mut source_lhs = replacement.context.inside.clone();
                        Self::replace_type_var(
                            &mut source_lhs,
                            &replacement.context.replaces,
                            e.outer(),
                        );
                        let mut source_rhs = replacement.context.inside.clone();
                        Self::replace_type_var(
                            &mut source_rhs,
                            &replacement.context.replaces,
                            g.outer(),
                        );
                        e.inside.replace(source_lhs);
                        g.inside.replace(source_rhs);
                        return Err(UnificationError::FromConstraints {
                            got: g,
                            expected: e,
                            source: replacement.context.source,
                            loc,
                        });
                    }
                    Err(
                        e @ UnificationError::FromConstraints { .. }
                        | e @ UnificationError::Specific { .. }
                        | e @ UnificationError::UnsatisfiedTraits(_, _),
                    ) => return Err(e),
                };
            }
        }

        Ok(new_type)
    }

    fn ensure_impls(
        &mut self,
        var: &TypeVar,
        traits: &TraitList,
        trait_is_expected: bool,
        ctx: &Context,
    ) -> std::result::Result<(), UnificationError> {
        self.trace_stack.push(TraceStackEntry::EnsuringImpls(
            var.clone(),
            traits.clone(),
            trait_is_expected,
        ));
        let number = ctx
            .symtab
            .lookup_trait(&Path::from_strs(&["Number"]).nowhere())
            .expect("Did not find number in symtab")
            .0;

        let mut error_producer = |required_traits| {
            if trait_is_expected {
                Err(UnificationError::Normal(Tm {
                    e: UnificationTrace::new(self.new_generic_with_traits(required_traits)),
                    g: UnificationTrace::new(var.clone()),
                }))
            } else {
                Err(UnificationError::Normal(Tm {
                    e: UnificationTrace::new(var.clone()),
                    g: UnificationTrace::new(self.new_generic_with_traits(required_traits)),
                }))
            }
        };

        match var {
            TypeVar::Known(_, KnownType::Named(name), _) => {
                let impld = ctx.items.impls.get(name);
                let unsatisfied = traits
                    .inner
                    .iter()
                    .filter(|t| {
                        // Number is special cased for now because we can't impl traits
                        // on generic types
                        if &t.name == &number {
                            let int_type = &ctx
                                .symtab
                                .lookup_type_symbol(&Path::from_strs(&["int"]).nowhere())
                                .expect("The type int was not in the symtab")
                                .0;
                            let uint_type = &ctx
                                .symtab
                                .lookup_type_symbol(&Path::from_strs(&["uint"]).nowhere())
                                .expect("The type uint was not in the symtab")
                                .0;

                            !(name == int_type || name == uint_type)
                        } else {
                            impld
                                .map(|impld| {
                                    impld.contains_key(&hir::TraitName::Named(
                                        t.name.clone().nowhere(),
                                    ))
                                })
                                .unwrap_or(true)
                        }
                    })
                    .cloned()
                    .collect::<Vec<_>>();

                if unsatisfied.is_empty() {
                    Ok(())
                } else {
                    error_producer(TraitList::from_vec(unsatisfied.clone()))
                }
            }
            TypeVar::Unknown(_, _) => {
                panic!("running ensure_impls on an unknown type")
            }
            _ => {
                if traits.inner.is_empty() {
                    Ok(())
                } else {
                    error_producer(traits.clone())
                }
            }
        }
    }

    fn replace_type_var(in_var: &mut TypeVar, from: &TypeVar, replacement: &TypeVar) {
        // First, do recursive replacement
        match in_var {
            TypeVar::Known(_, _, params) => {
                for param in params {
                    Self::replace_type_var(param, from, replacement)
                }
            }
            TypeVar::Unknown(_, traits) => {
                for t in traits.inner.iter_mut() {
                    for param in t.type_params.iter_mut() {
                        Self::replace_type_var(param, from, replacement)
                    }
                }
            }
        }

        let is_same = match (&in_var, &from) {
            // Traits do not matter for comparison
            (TypeVar::Unknown(id1, _), TypeVar::Unknown(id2, _)) => id1 == id2,
            (l, r) => l == r,
        };

        if is_same {
            *in_var = replacement.clone();
        }
    }

    fn replace_type_var_in_constraint_expr(
        in_constraint: &mut ConstraintExpr,
        from: &TypeVar,
        replacement: &TypeVar,
    ) {
        match in_constraint {
            ConstraintExpr::Integer(_) => {}
            ConstraintExpr::Var(v) => {
                Self::replace_type_var(v, from, replacement);

                match v {
                    TypeVar::Known(_, KnownType::Integer(val), _) => {
                        *in_constraint = ConstraintExpr::Integer(val.clone().to_bigint())
                    }
                    _ => {}
                }
            }
            ConstraintExpr::Sum(lhs, rhs) => {
                Self::replace_type_var_in_constraint_expr(lhs, from, replacement);
                Self::replace_type_var_in_constraint_expr(rhs, from, replacement);
            }
            ConstraintExpr::Sub(i) | ConstraintExpr::BitsToRepresent(i) => {
                Self::replace_type_var_in_constraint_expr(i, from, replacement);
            }
        }
    }

    fn replace_type_var_in_constraint_rhs(
        in_constraint: &mut ConstraintRhs,
        from: &TypeVar,
        replacement: &TypeVar,
    ) {
        Self::replace_type_var_in_constraint_expr(&mut in_constraint.constraint, from, replacement);
        // NOTE: We do not want to replace type variables here as that that removes
        // information about where the constraint relates. Instead, this replacement
        // is performed when reporting the error
        // Self::replace_type_var(&mut in_constraint.from, from, replacement);
    }

    pub fn unify_expression_generic_error<'a>(
        &mut self,
        expr: &Loc<Expression>,
        other: &impl HasType,
        ctx: &Context,
    ) -> Result<TypeVar> {
        self.unify(&expr.inner, other, ctx)
            .into_default_diagnostic(expr.loc())
    }

    pub fn check_requirements(&mut self, ctx: &Context) -> Result<()> {
        // Once we are done type checking the rest of the entity, check all requirements
        loop {
            // Walk through all the requirements, checking each one. If the requirement
            // is still undetermined, take note to retain that id, otherwise store the
            // replacement to be performed
            let (retain, replacements_option): (Vec<_>, Vec<_>) = self
                .requirements
                .clone()
                .iter()
                .map(|req| match req.check(self, ctx)? {
                    requirements::RequirementResult::NoChange => Ok((true, None)),
                    requirements::RequirementResult::Satisfied(replacement) => {
                        self.trace_stack
                            .push(TraceStackEntry::ResolvedRequirement(req.clone()));
                        Ok((false, Some(replacement)))
                    }
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .unzip();

            let replacements = replacements_option
                .into_iter()
                .flatten()
                .flatten()
                .collect::<Vec<_>>();
            if replacements.is_empty() {
                break;
            }

            for Replacement { from, to, context } in replacements {
                self.unify(&to, &from, ctx)
                    .into_diagnostic_or_default(from.loc(), context)?;
            }

            // Drop all replacements that have now been applied
            self.requirements = self
                .requirements
                .drain(0..)
                .zip(retain)
                .filter_map(|(req, keep)| if keep { Some(req) } else { None })
                .collect();
        }

        Ok(())
    }
}

impl TypeState {
    pub fn print_equations(&self) {
        for (lhs, rhs) in &self.equations {
            println!(
                "{} -> {}",
                format!("{lhs}").blue(),
                format!("{rhs:?}").green()
            )
        }

        println!("\nReplacments:");

        for (lhs, rhs) in self.replacements.iter().sorted() {
            println!(
                "{} -> {}",
                format!("{lhs:?}").blue(),
                format!("{rhs:?}").green()
            )
        }

        println!("\n Requirements:");

        for requirement in &self.requirements {
            println!("{:?}", requirement)
        }

        println!("")
    }
}

pub trait HasType: std::fmt::Debug {
    fn get_type(&self, state: &TypeState) -> Result<TypeVar>;
}

impl HasType for TypeVar {
    fn get_type(&self, state: &TypeState) -> Result<TypeVar> {
        Ok(state.check_var_for_replacement(self.clone()))
    }
}
impl HasType for Loc<TypeVar> {
    fn get_type(&self, state: &TypeState) -> Result<TypeVar> {
        self.inner.get_type(state)
    }
}
impl HasType for TypedExpression {
    fn get_type(&self, state: &TypeState) -> Result<TypeVar> {
        state.type_of(self)
    }
}
impl HasType for Expression {
    fn get_type(&self, state: &TypeState) -> Result<TypeVar> {
        state.type_of(&TypedExpression::Id(self.id))
    }
}
impl HasType for Loc<Expression> {
    fn get_type(&self, state: &TypeState) -> Result<TypeVar> {
        state.type_of(&TypedExpression::Id(self.inner.id))
    }
}
impl HasType for Pattern {
    fn get_type(&self, state: &TypeState) -> Result<TypeVar> {
        state.type_of(&TypedExpression::Id(self.id))
    }
}
impl HasType for Loc<Pattern> {
    fn get_type(&self, state: &TypeState) -> Result<TypeVar> {
        state.type_of(&TypedExpression::Id(self.inner.id))
    }
}
impl HasType for Loc<KnownType> {
    fn get_type(&self, _state: &TypeState) -> Result<TypeVar> {
        Ok(TypeVar::Known(self.loc(), self.inner.clone(), vec![]))
    }
}
impl HasType for NameID {
    fn get_type(&self, state: &TypeState) -> Result<TypeVar> {
        state.type_of(&TypedExpression::Name(self.clone()))
    }
}

/// Mapping between names and concrete type used for lookup, without being
/// able to do more type inference
/// Required because we can't serde the whole TypeState
#[derive(Serialize, Deserialize)]
pub struct TypeMap {
    equations: TypeEquations,
}

impl TypeMap {
    pub fn type_of(&self, thing: &TypedExpression) -> Option<&TypeVar> {
        self.equations.get(thing)
    }
}

impl From<TypeState> for TypeMap {
    fn from(val: TypeState) -> Self {
        TypeMap {
            equations: val.equations,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::TypeVar as TVar;
    use super::TypedExpression as TExpr;

    use crate::testutil::{sized_int, unsized_int};
    use crate::{ensure_same_type, get_type, HasType};
    use crate::{fixed_types::t_clock, hir};
    use hir::hparams;
    use hir::symbol_table::TypeDeclKind;
    use hir::PatternKind;
    use hir::{dtype, testutil::t_num, ArgumentList};
    use spade_ast::testutil::{ast_ident, ast_path};
    use spade_common::location_info::WithLocation;
    use spade_common::name::testutil::name_id;
    use spade_common::num_ext::InfallibleToBigInt;
    use spade_hir::symbol_table::{SymbolTable, Thing};

    #[test]
    fn if_statements_have_correctly_inferred_types() {
        let mut state = TypeState::new();
        let mut symtab = SymbolTable::new();
        spade_ast_lowering::builtins::populate_symtab(&mut symtab, &mut ItemList::new());

        let input = ExprKind::If(
            Box::new(Expression::ident(0, 0, "a").nowhere()),
            Box::new(Expression::ident(1, 1, "b").nowhere()),
            Box::new(Expression::ident(2, 2, "c").nowhere()),
        )
        .with_id(3)
        .nowhere();

        // Add eqs for the literals
        let expr_a = TExpr::Name(name_id(0, "a").inner);
        let expr_b = TExpr::Name(name_id(1, "b").inner);
        let expr_c = TExpr::Name(name_id(2, "c").inner);
        state.add_eq_from_tvar(expr_a.clone(), TVar::Unknown(100, TraitList::empty()));
        state.add_eq_from_tvar(expr_b.clone(), TVar::Unknown(101, TraitList::empty()));
        state.add_eq_from_tvar(expr_c.clone(), TVar::Unknown(102, TraitList::empty()));

        let ctx = Context {
            symtab: &symtab,
            items: &ItemList::new(),
        };
        let generic_list = state
            .create_generic_list(GenericListSource::Anonymous, &vec![], None)
            .unwrap();
        state.visit_expression(&input, &ctx, &generic_list).unwrap();

        // Check the generic type variables
        ensure_same_type!(
            state,
            TExpr::Id(0),
            TVar::Known(().nowhere(), t_bool(&symtab), vec![])
        );
        ensure_same_type!(state, TExpr::Id(1), TExpr::Id(2));
        ensure_same_type!(state, TExpr::Id(1), TExpr::Id(3));

        // Check the constraints added to the literals
        ensure_same_type!(state, TExpr::Id(0), expr_a);
        ensure_same_type!(state, TExpr::Id(1), expr_b);
        ensure_same_type!(state, TExpr::Id(2), expr_c);
    }

    #[test]
    fn if_expressions_get_correct_type_when_branches_are_of_known_type() {
        let mut state = TypeState::new();
        let mut symtab = SymbolTable::new();
        spade_ast_lowering::builtins::populate_symtab(&mut symtab, &mut ItemList::new());

        let input = ExprKind::If(
            Box::new(Expression::ident(0, 0, "a").nowhere()),
            Box::new(Expression::ident(1, 1, "b").nowhere()),
            Box::new(Expression::ident(2, 2, "c").nowhere()),
        )
        .with_id(3)
        .nowhere();

        // Add eqs for the literals
        let expr_a = TExpr::Name(name_id(0, "a").inner);
        let expr_b = TExpr::Name(name_id(1, "b").inner);
        let expr_c = TExpr::Name(name_id(2, "c").inner);
        state.add_eq_from_tvar(expr_a.clone(), TVar::Unknown(100, TraitList::empty()));
        state.add_eq_from_tvar(expr_b.clone(), unsized_int(101, &symtab));
        state.add_eq_from_tvar(expr_c.clone(), TVar::Unknown(102, TraitList::empty()));
        let ctx = Context {
            symtab: &symtab,
            items: &ItemList::new(),
        };
        let generic_list = state
            .create_generic_list(GenericListSource::Anonymous, &vec![], None)
            .unwrap();
        state.visit_expression(&input, &ctx, &generic_list).unwrap();

        // Check the generic type variables
        ensure_same_type!(
            state,
            TExpr::Id(0),
            TVar::Known(().nowhere(), t_bool(&symtab), vec![])
        );
        ensure_same_type!(state, TExpr::Id(1), unsized_int(101, &symtab));
        ensure_same_type!(state, TExpr::Id(2), unsized_int(101, &symtab));
        ensure_same_type!(state, TExpr::Id(3), unsized_int(101, &symtab));

        // Check the constraints added to the literals
        ensure_same_type!(state, TExpr::Id(0), expr_a);
        ensure_same_type!(state, TExpr::Id(1), expr_b);
        ensure_same_type!(state, TExpr::Id(2), expr_c);
    }

    #[test]
    fn type_inference_fails_if_if_branches_have_incompatible_types() {
        let mut state = TypeState::new();
        let mut symtab = SymbolTable::new();
        spade_ast_lowering::builtins::populate_symtab(&mut symtab, &mut ItemList::new());

        let input = ExprKind::If(
            Box::new(Expression::ident(0, 0, "a").nowhere()),
            Box::new(Expression::ident(1, 1, "b").nowhere()),
            Box::new(Expression::ident(2, 2, "c").nowhere()),
        )
        .with_id(3)
        .nowhere();

        // Add eqs for the literals
        let expr_a = TExpr::Name(name_id(0, "a").inner);
        let expr_b = TExpr::Name(name_id(1, "b").inner);
        let expr_c = TExpr::Name(name_id(2, "c").inner);
        state.add_eq_from_tvar(expr_a.clone(), TVar::Unknown(100, TraitList::empty()));
        state.add_eq_from_tvar(expr_b.clone(), unsized_int(101, &symtab));
        state.add_eq_from_tvar(
            expr_c.clone(),
            TVar::Known(().nowhere(), t_clock(&symtab), vec![]),
        );

        let ctx = Context {
            symtab: &symtab,
            items: &ItemList::new(),
        };
        let generic_list = state
            .create_generic_list(GenericListSource::Anonymous, &vec![], None)
            .unwrap();
        assert_ne!(state.visit_expression(&input, &ctx, &generic_list), Ok(()));
    }

    #[test]
    fn match_expressions_have_correctly_inferred_types() {
        let mut state = TypeState::new();
        let mut symtab = SymbolTable::new();
        spade_ast_lowering::builtins::populate_symtab(&mut symtab, &mut ItemList::new());

        let first_pattern = PatternKind::Tuple(vec![
            PatternKind::name(name_id(10, "x1")).with_id(20).nowhere(),
            PatternKind::name(name_id(11, "x2")).with_id(21).nowhere(),
        ])
        .with_id(22)
        .nowhere();

        let input = ExprKind::Match(
            Box::new(Expression::ident(0, 0, "a").nowhere()),
            vec![
                (first_pattern, Expression::ident(1, 1, "b").nowhere()),
                (
                    PatternKind::name(name_id(12, "y")).with_id(23).nowhere(),
                    Expression::ident(2, 2, "c").nowhere(),
                ),
            ],
        )
        .with_id(3)
        .nowhere();

        // Add eqs for the literals
        let expr_a = TExpr::Name(name_id(0, "a").inner);
        let expr_b = TExpr::Name(name_id(1, "b").inner);
        let expr_c = TExpr::Name(name_id(2, "c").inner);
        state.add_eq_from_tvar(expr_a.clone(), TVar::Unknown(100, TraitList::empty()));
        state.add_eq_from_tvar(expr_b.clone(), unsized_int(101, &symtab));
        state.add_eq_from_tvar(expr_c.clone(), TVar::Unknown(102, TraitList::empty()));

        let ctx = Context {
            symtab: &symtab,
            items: &ItemList::new(),
        };
        let generic_list = state
            .create_generic_list(GenericListSource::Anonymous, &vec![], None)
            .unwrap();
        state.visit_expression(&input, &ctx, &generic_list).unwrap();

        // Ensure branches have the same type
        ensure_same_type!(state, expr_b, &expr_c);
        // And that the match block has the same type
        ensure_same_type!(state, expr_c, TExpr::Id(3));

        // Ensure patterns have same type as each other, and as the expression
        ensure_same_type!(state, expr_a, TExpr::Id(22));
        ensure_same_type!(state, TExpr::Id(22), TExpr::Id(23));
    }

    #[test]
    fn patterns_constrain_expression_types() {
        let mut state = TypeState::new();
        let mut symtab = SymbolTable::new();
        spade_ast_lowering::builtins::populate_symtab(&mut symtab, &mut ItemList::new());

        let first_pattern = PatternKind::Tuple(vec![
            PatternKind::Bool(true).with_id(20).nowhere(),
            PatternKind::Bool(true).with_id(21).nowhere(),
        ])
        .with_id(22)
        .nowhere();

        let input = ExprKind::Match(
            Box::new(Expression::ident(0, 0, "a").nowhere()),
            vec![
                (first_pattern, Expression::ident(1, 1, "b").nowhere()),
                (
                    PatternKind::name(name_id(11, "y")).with_id(23).nowhere(),
                    Expression::ident(2, 2, "c").nowhere(),
                ),
            ],
        )
        .with_id(3)
        .nowhere();

        // Add eqs for the literals
        let expr_a = TExpr::Name(name_id(0, "a").inner);
        let expr_b = TExpr::Name(name_id(1, "b").inner);
        let expr_c = TExpr::Name(name_id(2, "c").inner);
        state.add_eq_from_tvar(expr_a.clone(), TVar::Unknown(100, TraitList::empty()));
        state.add_eq_from_tvar(expr_b.clone(), TVar::Unknown(101, TraitList::empty()));
        state.add_eq_from_tvar(expr_c.clone(), TVar::Unknown(102, TraitList::empty()));

        let ctx = Context {
            symtab: &symtab,
            items: &ItemList::new(),
        };
        let generic_list = state
            .create_generic_list(GenericListSource::Anonymous, &vec![], None)
            .unwrap();
        state.visit_expression(&input, &ctx, &generic_list).unwrap();

        let expected_type = TVar::tuple(
            ().nowhere(),
            vec![kvar!(t_bool(&symtab)), kvar!(t_bool(&symtab))],
        );

        // Ensure patterns have same type as each other, and as the expression
        ensure_same_type!(state, expr_a, expected_type);
    }

    #[test]
    fn not_all_types_are_the_same() {
        let mut state = TypeState::new();
        let mut symtab = SymbolTable::new();
        spade_ast_lowering::builtins::populate_symtab(&mut symtab, &mut ItemList::new());

        let not_bool = symtab.add_type_with_id(
            100,
            ast_path("not_bool").inner,
            TypeSymbol::Declared(vec![], TypeDeclKind::Enum).nowhere(),
        );

        let lhs = PatternKind::name(name_id(0, "x")).with_id(22).nowhere();

        state.add_eq_from_tvar(
            TExpr::Name(name_id(1, "a").inner),
            kvar!(KnownType::Named(not_bool)),
        );

        let input = Statement::binding(
            lhs,
            Some(dtype!(symtab => "bool")),
            Expression::ident(1, 1, "a").nowhere(),
        )
        .nowhere();
        let ctx = Context {
            symtab: &symtab,
            items: &ItemList::new(),
        };
        let generic_list = state
            .create_generic_list(GenericListSource::Anonymous, &vec![], None)
            .unwrap();
        assert!(state.visit_statement(&input, &ctx, &generic_list).is_err());
    }

    #[test]
    fn integer_literals_are_compatible_with_fixed_size_ints() {
        let mut state = TypeState::new();
        let mut symtab = SymbolTable::new();
        spade_ast_lowering::builtins::populate_symtab(&mut symtab, &mut ItemList::new());

        let input = ExprKind::If(
            Box::new(Expression::ident(0, 0, "a").nowhere()),
            Box::new(Expression::ident(1, 1, "b").nowhere()),
            Box::new(Expression::ident(2, 2, "c").nowhere()),
        )
        .with_id(3)
        .nowhere();

        // Add eqs for the literals
        let expr_a = TExpr::Name(name_id(0, "a").inner);
        let expr_b = TExpr::Name(name_id(1, "b").inner);
        let expr_c = TExpr::Name(name_id(2, "c").inner);
        state.add_eq_from_tvar(expr_a.clone(), TVar::Unknown(100, TraitList::empty()));
        state.add_eq_from_tvar(expr_b.clone(), unsized_int(101, &symtab));
        state.add_eq_from_tvar(expr_c.clone(), sized_int(5, &symtab));

        let ctx = Context {
            symtab: &symtab,
            items: &ItemList::new(),
        };
        let generic_list = state
            .create_generic_list(GenericListSource::Anonymous, &vec![], None)
            .unwrap();
        state.visit_expression(&input, &ctx, &generic_list).unwrap();

        // Check the generic type variables
        ensure_same_type!(
            state,
            TExpr::Id(0),
            TVar::Known(().nowhere(), t_bool(&symtab), vec![])
        );
        ensure_same_type!(state, TExpr::Id(1), sized_int(5, &symtab));
        ensure_same_type!(state, TExpr::Id(2), sized_int(5, &symtab));
        ensure_same_type!(state, TExpr::Id(3), sized_int(5, &symtab));

        // Check the constraints added to the literals
        ensure_same_type!(state, TExpr::Id(0), expr_a);
        ensure_same_type!(state, TExpr::Id(1), expr_b);
        ensure_same_type!(state, TExpr::Id(2), expr_c);
    }

    #[test]
    fn typed_let_bindings_typecheck_correctly() {
        let mut symtab = SymbolTable::new();
        spade_ast_lowering::builtins::populate_symtab(&mut symtab, &mut ItemList::new());

        let input = hir::Statement::binding(
            PatternKind::name(name_id(0, "a")).with_id(10).nowhere(),
            Some(dtype!(symtab => "int"; (t_num(5)))),
            ExprKind::IntLiteral(0.to_bigint()).with_id(0).nowhere(),
        )
        .nowhere();

        let mut state = TypeState::new();

        let ctx = Context {
            symtab: &symtab,
            items: &ItemList::new(),
        };
        let generic_list = state
            .create_generic_list(GenericListSource::Anonymous, &vec![], None)
            .unwrap();
        state.visit_statement(&input, &ctx, &generic_list).unwrap();

        let ta = get_type!(state, &TExpr::Name(name_id(0, "a").inner));
        ensure_same_type!(state, ta, sized_int(5, &symtab));
    }

    #[test]
    fn tuple_type_specs_propagate_correctly() {
        let mut symtab = SymbolTable::new();
        spade_ast_lowering::builtins::populate_symtab(&mut symtab, &mut ItemList::new());

        let input = Register {
            pattern: PatternKind::name(name_id(0, "test")).with_id(10).nowhere(),
            clock: ExprKind::Identifier(name_id(1, "clk").inner)
                .with_id(0)
                .nowhere(),
            reset: None,
            initial: None,
            value: ExprKind::TupleLiteral(vec![
                ExprKind::IntLiteral(5.to_bigint()).with_id(1).nowhere(),
                ExprKind::BoolLiteral(true).with_id(2).nowhere(),
            ])
            .with_id(3)
            .nowhere(),
            value_type: Some(
                hir::TypeSpec::Tuple(vec![
                    dtype!(symtab => "int"; ( t_num(5) )),
                    hir::dtype!(symtab => "bool"),
                ])
                .nowhere(),
            ),
            attributes: hir::AttributeList::empty(),
        };

        let mut state = TypeState::new();

        let expr_clk = TExpr::Name(name_id(1, "clk").inner);
        state.add_eq_from_tvar(expr_clk.clone(), TVar::Unknown(100, TraitList::empty()));

        let ctx = Context {
            symtab: &symtab,
            items: &ItemList::new(),
        };
        let generic_list = state
            .create_generic_list(GenericListSource::Anonymous, &vec![], None)
            .unwrap();
        state.visit_register(&input, &ctx, &generic_list).unwrap();

        let ttup = get_type!(state, &TExpr::Id(3));
        let reg = get_type!(state, &TExpr::Name(name_id(0, "test").inner));
        let expected = TypeVar::tuple(
            ().nowhere(),
            vec![
                sized_int(5, &symtab),
                TypeVar::Known(().nowhere(), t_bool(&symtab), vec![]),
            ],
        );
        ensure_same_type!(state, ttup, &expected);
        ensure_same_type!(state, reg, &expected);
    }

    #[test]
    fn entity_type_inference_works() {
        // Add the head to the symtab
        let mut symtab = SymbolTable::new();
        spade_ast_lowering::builtins::populate_symtab(&mut symtab, &mut ItemList::new());

        // Add the entity to the symtab
        let unit = hir::UnitHead {
            name: Identifier("".to_string()).nowhere(),
            unit_kind: hir::UnitKind::Entity.nowhere(),
            inputs: hparams![
                ("a", dtype!(symtab => "bool")),
                ("b", dtype!(symtab => "int"; (t_num(10)))),
            ]
            .nowhere(),
            output_type: Some(dtype!(symtab => "int"; (t_num(5)))),
            type_params: vec![],
        };

        let entity_name = symtab.add_thing(ast_path("test").inner, Thing::Unit(unit.nowhere()));

        let input = ExprKind::Call {
            kind: hir::expression::CallKind::Entity(().nowhere()),
            callee: entity_name.nowhere(),
            args: ArgumentList::Positional(vec![
                Expression::ident(0, 0, "x").nowhere(),
                Expression::ident(1, 1, "b").nowhere(),
            ])
            .nowhere(),
            turbofish: None,
        }
        .with_id(2)
        .nowhere();

        let mut state = TypeState::new();

        // Add eqs for the literals
        let expr_a = TExpr::Name(name_id(0, "a").inner);
        let expr_b = TExpr::Name(name_id(1, "b").inner);
        state.add_eq_from_tvar(expr_a.clone(), TVar::Unknown(100, TraitList::empty()));
        state.add_eq_from_tvar(expr_b.clone(), TVar::Unknown(101, TraitList::empty()));

        let generic_list = state
            .create_generic_list(GenericListSource::Anonymous, &vec![], None)
            .unwrap();
        state
            .visit_expression(
                &input,
                &Context {
                    symtab: &symtab,
                    items: &ItemList::new(),
                },
                &generic_list,
            )
            .unwrap();

        let t0 = get_type!(state, &TExpr::Id(0));
        let t1 = get_type!(state, &TExpr::Id(1));
        let t2 = get_type!(state, &TExpr::Id(2));

        let ta = get_type!(state, &expr_a);
        let tb = get_type!(state, &expr_b);

        // Check the generic type variables
        ensure_same_type!(
            state,
            t0.clone(),
            TVar::Known(().nowhere(), t_bool(&symtab), vec![])
        );
        ensure_same_type!(
            state,
            t1.clone(),
            TVar::Known(
                ().nowhere(),
                t_int(&symtab),
                vec![TypeVar::Known(().nowhere(), KnownType::integer(10), vec![])],
            )
        );
        ensure_same_type!(
            state,
            t2,
            TVar::Known(
                ().nowhere(),
                t_int(&symtab),
                vec![TypeVar::Known(().nowhere(), KnownType::integer(5), vec![])],
            )
        );

        // Check the constraints added to the literals
        ensure_same_type!(state, t0.clone(), ta);
        ensure_same_type!(state, t1.clone(), tb);
    }

    #[test]
    fn pipeline_type_inference_works() {
        // Add the head to the symtab
        let mut symtab = SymbolTable::new();
        spade_ast_lowering::builtins::populate_symtab(&mut symtab, &mut ItemList::new());

        // Add the entity to the symtab
        let unit = hir::UnitHead {
            name: Identifier("".to_string()).nowhere(),
            unit_kind: hir::UnitKind::Pipeline(2.nowhere()).nowhere(),
            inputs: hparams![
                ("a", dtype!(symtab => "bool")),
                ("b", dtype!(symtab => "int"; ( t_num(10) ))),
            ]
            .nowhere(),
            output_type: Some(dtype!(symtab => "int"; ( t_num(5) ))),
            type_params: vec![],
        };

        let entity_name = symtab.add_thing(ast_path("test").inner, Thing::Unit(unit.nowhere()));

        let input = ExprKind::Call {
            kind: hir::expression::CallKind::Pipeline(().nowhere(), 2.nowhere()),
            callee: entity_name.nowhere(),
            args: ArgumentList::Positional(vec![
                Expression::ident(0, 0, "x").nowhere(),
                Expression::ident(1, 1, "b").nowhere(),
            ])
            .nowhere(),
            turbofish: None,
        }
        .with_id(2)
        .nowhere();

        let mut state = TypeState::new();

        // Add eqs for the literals
        let expr_a = TExpr::Name(name_id(0, "a").inner);
        let expr_b = TExpr::Name(name_id(1, "b").inner);
        state.add_eq_from_tvar(expr_a.clone(), TVar::Unknown(100, TraitList::empty()));
        state.add_eq_from_tvar(expr_b.clone(), TVar::Unknown(101, TraitList::empty()));

        let generic_list = state
            .create_generic_list(GenericListSource::Anonymous, &vec![], None)
            .unwrap();
        state
            .visit_expression(
                &input,
                &Context {
                    symtab: &symtab,
                    items: &ItemList::new(),
                },
                &generic_list,
            )
            .unwrap();

        // Check the generic type variables
        ensure_same_type!(
            state,
            TExpr::Id(0),
            TVar::Known(().nowhere(), t_bool(&symtab), vec![])
        );
        ensure_same_type!(
            state,
            TExpr::Id(1),
            TVar::Known(
                ().nowhere(),
                t_int(&symtab),
                vec![TypeVar::Known(().nowhere(), KnownType::integer(10), vec![])],
            )
        );
        ensure_same_type!(
            state,
            TExpr::Id(2),
            TVar::Known(
                ().nowhere(),
                t_int(&symtab),
                vec![TypeVar::Known(().nowhere(), KnownType::integer(5), vec![])],
            )
        );

        // Check the constraints added to the literals
        ensure_same_type!(state, &TExpr::Id(0), &expr_a);
        ensure_same_type!(state, &TExpr::Id(1), &expr_b);
    }
}
