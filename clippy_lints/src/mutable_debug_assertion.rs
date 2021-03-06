use crate::utils::{is_direct_expn_of, span_lint};
use if_chain::if_chain;
use rustc::hir::map::Map;
use rustc::ty;
use rustc_hir::intravisit::{walk_expr, NestedVisitorMap, Visitor};
use rustc_hir::{BorrowKind, Expr, ExprKind, MatchSource, Mutability, StmtKind, UnOp};
use rustc_lint::{LateContext, LateLintPass};
use rustc_session::{declare_lint_pass, declare_tool_lint};
use rustc_span::Span;

declare_clippy_lint! {
    /// **What it does:** Checks for function/method calls with a mutable
    /// parameter in `debug_assert!`, `debug_assert_eq!` and `debug_assert_ne!` macros.
    ///
    /// **Why is this bad?** In release builds `debug_assert!` macros are optimized out by the
    /// compiler.
    /// Therefore mutating something in a `debug_assert!` macro results in different behaviour
    /// between a release and debug build.
    ///
    /// **Known problems:** None
    ///
    /// **Example:**
    /// ```rust,ignore
    /// debug_assert_eq!(vec![3].pop(), Some(3));
    /// // or
    /// fn take_a_mut_parameter(_: &mut u32) -> bool { unimplemented!() }
    /// debug_assert!(take_a_mut_parameter(&mut 5));
    /// ```
    pub DEBUG_ASSERT_WITH_MUT_CALL,
    nursery,
    "mutable arguments in `debug_assert{,_ne,_eq}!`"
}

declare_lint_pass!(DebugAssertWithMutCall => [DEBUG_ASSERT_WITH_MUT_CALL]);

const DEBUG_MACRO_NAMES: [&str; 3] = ["debug_assert", "debug_assert_eq", "debug_assert_ne"];

impl<'a, 'tcx> LateLintPass<'a, 'tcx> for DebugAssertWithMutCall {
    fn check_expr(&mut self, cx: &LateContext<'a, 'tcx>, e: &'tcx Expr<'_>) {
        for dmn in &DEBUG_MACRO_NAMES {
            if is_direct_expn_of(e.span, dmn).is_some() {
                if let Some(span) = extract_call(cx, e) {
                    span_lint(
                        cx,
                        DEBUG_ASSERT_WITH_MUT_CALL,
                        span,
                        &format!("do not call a function with mutable arguments inside of `{}!`", dmn),
                    );
                }
            }
        }
    }
}

//HACK(hellow554): remove this when #4694 is implemented
#[allow(clippy::cognitive_complexity)]
fn extract_call<'a, 'tcx>(cx: &'a LateContext<'a, 'tcx>, e: &'tcx Expr<'_>) -> Option<Span> {
    if_chain! {
        if let ExprKind::Block(ref block, _) = e.kind;
        if block.stmts.len() == 1;
        if let StmtKind::Semi(ref matchexpr) = block.stmts[0].kind;
        then {
            // debug_assert
            if_chain! {
                if let ExprKind::Match(ref ifclause, _, _) = matchexpr.kind;
                if let ExprKind::DropTemps(ref droptmp) = ifclause.kind;
                if let ExprKind::Unary(UnOp::UnNot, ref condition) = droptmp.kind;
                then {
                    let mut visitor = MutArgVisitor::new(cx);
                    visitor.visit_expr(condition);
                    return visitor.expr_span();
                }
            }

            // debug_assert_{eq,ne}
            if_chain! {
                if let ExprKind::Block(ref matchblock, _) = matchexpr.kind;
                if let Some(ref matchheader) = matchblock.expr;
                if let ExprKind::Match(ref headerexpr, _, _) = matchheader.kind;
                if let ExprKind::Tup(ref conditions) = headerexpr.kind;
                if conditions.len() == 2;
                then {
                    if let ExprKind::AddrOf(BorrowKind::Ref, _, ref lhs) = conditions[0].kind {
                        let mut visitor = MutArgVisitor::new(cx);
                        visitor.visit_expr(lhs);
                        if let Some(span) = visitor.expr_span() {
                            return Some(span);
                        }
                    }
                    if let ExprKind::AddrOf(BorrowKind::Ref, _, ref rhs) = conditions[1].kind {
                        let mut visitor = MutArgVisitor::new(cx);
                        visitor.visit_expr(rhs);
                        if let Some(span) = visitor.expr_span() {
                            return Some(span);
                        }
                    }
                }
            }
        }
    }

    None
}

struct MutArgVisitor<'a, 'tcx> {
    cx: &'a LateContext<'a, 'tcx>,
    expr_span: Option<Span>,
    found: bool,
}

impl<'a, 'tcx> MutArgVisitor<'a, 'tcx> {
    fn new(cx: &'a LateContext<'a, 'tcx>) -> Self {
        Self {
            cx,
            expr_span: None,
            found: false,
        }
    }

    fn expr_span(&self) -> Option<Span> {
        if self.found {
            self.expr_span
        } else {
            None
        }
    }
}

impl<'a, 'tcx> Visitor<'tcx> for MutArgVisitor<'a, 'tcx> {
    type Map = Map<'tcx>;

    fn visit_expr(&mut self, expr: &'tcx Expr<'_>) {
        match expr.kind {
            ExprKind::AddrOf(BorrowKind::Ref, Mutability::Mut, _) => {
                self.found = true;
                return;
            },
            ExprKind::Path(_) => {
                if let Some(adj) = self.cx.tables.adjustments().get(expr.hir_id) {
                    if adj
                        .iter()
                        .any(|a| matches!(a.target.kind, ty::Ref(_, _, Mutability::Mut)))
                    {
                        self.found = true;
                        return;
                    }
                }
            },
            // Don't check await desugars
            ExprKind::Match(_, _, MatchSource::AwaitDesugar) => return,
            _ if !self.found => self.expr_span = Some(expr.span),
            _ => return,
        }
        walk_expr(self, expr)
    }

    fn nested_visit_map(&mut self) -> NestedVisitorMap<Self::Map> {
        NestedVisitorMap::OnlyBodies(self.cx.tcx.hir())
    }
}
