use spade_common::{
    location_info::{Loc, WithLocation},
    name::Identifier,
};
use spade_diagnostics::Diagnostic;
use spade_hir::{
    expression::NamedArgument, symbol_table::FrozenSymtab, ArgumentList, Expression, ItemList,
};
use spade_typeinference::{method_resolution::select_method, HasType, TypeState};

use crate::passes::pass::Pass;

pub struct LowerMethods<'a> {
    pub type_state: &'a TypeState,
    pub items: &'a ItemList,
    pub symtab: &'a FrozenSymtab,
}

impl<'a> Pass for LowerMethods<'a> {
    fn visit_expression(&mut self, expression: &mut Loc<Expression>) -> crate::error::Result<()> {
        let replacement_kind = match &mut expression.kind {
            spade_hir::ExprKind::MethodCall {
                target: self_,
                name,
                args,
                call_kind,
                // Turbofishes are only important during type inference
                turbofish: _,
            } => {
                let self_type = self_.get_type(self.type_state).map_err(|e| {
                    Diagnostic::bug(self_.as_ref(), format!("did not find a type ({e})"))
                })?;

                let Some(method) = select_method(self_.loc(), &self_type, name, self.items)? else {
                    return Err(Diagnostic::bug(
                        expression.loc(),
                        format!("Ambiguous method call. Multiple candidates exist for {self_type}"),
                    ));
                };

                let unit = self.symtab.symtab().unit_by_id(&method.inner);

                // Insert self as the first arg
                let args = args.map_ref(|args| {
                    let mut new = args.clone();
                    match &mut new {
                        ArgumentList::Named(list) => list.push(NamedArgument::Full(
                            Identifier(String::from("self")).nowhere(),
                            self_.as_ref().clone(),
                        )),
                        ArgumentList::Positional(list) => list.insert(0, self_.as_ref().clone()),
                    }
                    new
                });

                match unit.unit_kind.inner {
                    spade_hir::UnitKind::Function(_) => {}
                    spade_hir::UnitKind::Entity => {}
                    spade_hir::UnitKind::Pipeline(_) => {
                        return Err(Diagnostic::error(
                            expression.loc(),
                            "Pipeline methods cannot be instantiated",
                        )
                        .secondary_label(&unit.unit_kind, "This is a pipeline")
                        .note("This restriction will be lifted in the future")
                        .into())
                    }
                }
                Some(spade_hir::ExprKind::Call {
                    kind: call_kind.clone(),
                    callee: method.inner.at_loc(name),
                    args: args.clone(),
                    turbofish: None,
                })
            }
            _ => None,
        };

        match replacement_kind {
            Some(new) => expression.kind = new,
            None => {}
        }
        Ok(())
    }

    fn visit_unit(&mut self, _unit: &mut spade_hir::Unit) -> crate::error::Result<()> {
        Ok(())
    }
}
