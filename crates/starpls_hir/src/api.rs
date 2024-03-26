use crate::{
    def::{
        resolver::Resolver,
        scope::{self, module_scopes, ParameterDef},
        Function as HirDefFunction, LoadItemId, Stmt,
    },
    module, source_map,
    typeck::{
        builtins::BuiltinFunction, intrinsics::IntrinsicFunction, resolve_type_ref, ParamInner,
        Substitution, Tuple, Ty, TypeRef,
    },
    Db, DisplayWithDb, ExprId, Name, TyKind,
};
use starpls_common::{Diagnostic, Diagnostics, File};
use starpls_syntax::{
    ast::{self, AstNode, AstPtr, SyntaxNodePtr},
    SyntaxNode, TextSize,
};

pub use crate::typeck::{Field, Param};

pub fn diagnostics_for_file(db: &dyn Db, file: File) -> impl Iterator<Item = Diagnostic> {
    module_scopes::accumulated::<Diagnostics>(db, file).into_iter()
}

pub struct Semantics<'a> {
    db: &'a dyn Db,
}

impl<'a> Semantics<'a> {
    pub fn new(db: &'a dyn Db) -> Self {
        Self { db }
    }

    pub fn function_for_def(&self, file: File, stmt: ast::DefStmt) -> Option<Function> {
        let ptr = AstPtr::new(&ast::Statement::cast(stmt.syntax().clone())?);
        let stmt = source_map(self.db, file).stmt_map.get(&ptr)?;
        match &module(self.db, file)[*stmt] {
            Stmt::Def { func, .. } => Some((*func).into()),
            _ => None,
        }
    }

    pub fn resolve_type(&self, type_: &ast::NamedType) -> Option<Type> {
        Some(
            resolve_type_ref(self.db, &TypeRef::from_str_opt(type_.name()?.text()))
                .0
                .into(),
        )
    }

    pub fn resolve_call_expr(&self, file: File, expr: &ast::CallExpr) -> Option<Function> {
        let ty = self.type_of_expr(file, &expr.callee()?)?;
        Some(match ty.ty.kind() {
            TyKind::Function(func) => (*func).into(),
            TyKind::IntrinsicFunction(func, _) => (*func).into(),
            TyKind::BuiltinFunction(func) => (*func).into(),
            _ => return None,
        })
    }

    pub fn type_of_expr(&self, file: File, expr: &ast::Expression) -> Option<Type> {
        let ptr = AstPtr::new(expr);
        let expr = source_map(self.db, file).expr_map.get(&ptr)?;
        Some(self.db.infer_expr(file, *expr).into())
    }

    pub fn type_of_param(&self, file: File, param: &ast::Parameter) -> Option<Type> {
        let ptr = AstPtr::new(param);
        let param = source_map(self.db, file).param_map.get(&ptr)?;
        Some(self.db.infer_param(file, *param).into())
    }

    pub fn resolve_load_stmt(&self, file: File, load_stmt: &ast::LoadStmt) -> Option<File> {
        let ptr = AstPtr::new(&ast::Statement::Load(load_stmt.clone()));
        let stmt = source_map(self.db, file).stmt_map.get(&ptr)?;
        let load_stmt = match module(self.db, file)[*stmt] {
            Stmt::Load { load_stmt, .. } => load_stmt,
            _ => return None,
        };
        self.db.resolve_load_stmt(file, load_stmt)
    }

    pub fn scope_for_module(&self, file: File) -> SemanticsScope {
        let resolver = Resolver::new_for_module(self.db, file);
        SemanticsScope { resolver }
    }

    pub fn scope_for_expr(&self, file: File, expr: &ast::Expression) -> Option<SemanticsScope> {
        let ptr = AstPtr::new(expr);
        let expr = source_map(self.db, file).expr_map.get(&ptr)?;
        let resolver = Resolver::new_for_expr(self.db, file, *expr);
        Some(SemanticsScope { resolver })
    }

    pub fn scope_for_offset(&self, file: File, offset: TextSize) -> SemanticsScope {
        let resolver = Resolver::new_for_offset(self.db, file, offset);
        SemanticsScope { resolver }
    }

    pub fn resolve_call_expr_active_param(
        &self,
        file: File,
        expr: &ast::CallExpr,
        active_arg: usize,
    ) -> Option<usize> {
        let ptr = AstPtr::new(&ast::Expression::Call(expr.clone()));
        let expr = source_map(self.db, file).expr_map.get(&ptr)?;
        self.db
            .resolve_call_expr_active_param(file, *expr, active_arg)
    }
}

pub struct Variable {
    id: Option<ExprId>,
}

impl Variable {
    pub fn is_user_defined(&self) -> bool {
        self.id.is_some()
    }
}

pub struct LoadItem {
    id: LoadItemId,
}

pub enum ScopeDef {
    Function(Function),
    Variable(Variable),
    Parameter(Param),
    LoadItem(LoadItem),
}

impl ScopeDef {
    pub fn syntax_node_ptr(&self, db: &dyn Db, file: File) -> Option<SyntaxNodePtr> {
        let source_map = source_map(db, file);
        match self {
            ScopeDef::Function(Function(FunctionInner::HirDef(func))) => Some(func.ptr(db)),
            ScopeDef::Variable(Variable { id: Some(id) }) => source_map
                .expr_map_back
                .get(id)
                .map(|ptr| ptr.syntax_node_ptr()),
            ScopeDef::Parameter(param) => param.syntax_node_ptr(db),
            ScopeDef::LoadItem(LoadItem { id }) => source_map
                .load_item_map_back
                .get(id)
                .map(|ptr| ptr.syntax_node_ptr()),
            _ => None,
        }
    }

    pub fn to_load_item(
        &self,
        db: &dyn Db,
        file: File,
        root: &SyntaxNode,
    ) -> Option<ast::LoadItem> {
        let source_map = source_map(db, file);
        match self {
            ScopeDef::LoadItem(LoadItem { id }) => source_map
                .load_item_map_back
                .get(id)
                .and_then(|ptr| ptr.try_to_node(root)),
            _ => None,
        }
    }
}

impl From<scope::ScopeDef> for ScopeDef {
    fn from(value: scope::ScopeDef) -> Self {
        match value {
            scope::ScopeDef::Function(it) => ScopeDef::Function(it.into()),
            scope::ScopeDef::IntrinsicFunction(it) => ScopeDef::Function(it.into()),
            scope::ScopeDef::BuiltinFunction(it) => ScopeDef::Function(it.into()),
            scope::ScopeDef::Variable(it) => ScopeDef::Variable(Variable { id: Some(it.expr) }),
            scope::ScopeDef::BuiltinVariable(_) => ScopeDef::Variable(Variable { id: None }),
            scope::ScopeDef::Parameter(ParameterDef {
                func: parent,
                index,
            }) => ScopeDef::Parameter(Param(ParamInner::Param { parent, index })),
            scope::ScopeDef::LoadItem(it) => ScopeDef::LoadItem(LoadItem { id: it.load_item }),
        }
    }
}

pub struct SemanticsScope<'a> {
    resolver: Resolver<'a>,
}

impl SemanticsScope<'_> {
    pub fn names(&self) -> impl Iterator<Item = (Name, ScopeDef)> {
        self.resolver
            .names()
            .into_iter()
            .map(|(name, def)| (name, def.into()))
    }

    pub fn resolve_name(&self, name: &Name) -> Option<Vec<ScopeDef>> {
        self.resolver
            .resolve_name(&name)
            .map(|defs| defs.into_iter().map(|def| def.into()).collect())
    }
}

pub struct Type {
    ty: Ty,
}

impl Type {
    pub fn is_function(&self) -> bool {
        matches!(
            self.ty.kind(),
            TyKind::Function(_) | TyKind::BuiltinFunction(_) | TyKind::IntrinsicFunction(_, _)
        )
    }

    pub fn is_unknown(&self) -> bool {
        self.ty.kind() == &TyKind::Unknown
    }

    pub fn is_user_defined_function(&self) -> bool {
        matches!(self.ty.kind(), TyKind::Function(_))
    }

    pub fn params(&self, db: &dyn Db) -> Vec<(Param, Type)> {
        match self.ty.params(db) {
            Some(params) => params.map(|(param, ty)| (param, ty.into())).collect(),
            None => Vec::new(),
        }
    }

    pub fn doc(&self, db: &dyn Db) -> Option<String> {
        Some(match self.ty.kind() {
            TyKind::BuiltinFunction(func) => func.doc(db).clone(),
            TyKind::BuiltinType(type_) => type_.doc(db).clone(),
            TyKind::Function(func) => return func.doc(db).map(|doc| doc.to_string()),
            TyKind::IntrinsicFunction(func, _) => func.doc(db).clone(),
            _ => return None,
        })
    }

    pub fn fields(&self, db: &dyn Db) -> Vec<(Field, Type)> {
        let fields = match self.ty.fields(db) {
            Some(fields) => fields,
            None => return Vec::new(),
        };

        fields.map(|(name, ty)| (name, ty.into())).collect()
    }

    pub fn known_keys(&self) -> Option<&[Box<str>]> {
        self.ty.known_keys()
    }

    pub fn dict_value_ty(&self) -> Option<Type> {
        match self.ty.kind() {
            TyKind::Dict(_, value_ty, _) => Some(value_ty.clone().into()),
            _ => None,
        }
    }

    pub fn variable_tuple_element_ty(&self) -> Option<Type> {
        match self.ty.kind() {
            TyKind::Tuple(Tuple::Variable(ty)) => Some(ty.clone().into()),
            _ => None,
        }
    }
}

impl From<Ty> for Type {
    fn from(ty: Ty) -> Self {
        Self { ty }
    }
}

impl DisplayWithDb for Type {
    fn fmt(&self, db: &dyn Db, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.ty.fmt(db, f)
    }

    fn fmt_alt(&self, db: &dyn Db, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.ty.fmt_alt(db, f)
    }
}

pub struct Function(FunctionInner);

impl Function {
    pub fn name(&self, db: &dyn Db) -> Name {
        match self.0 {
            FunctionInner::HirDef(func) => func.name(db),
            FunctionInner::IntrinsicFunction(func) => func.name(db),
            FunctionInner::BuiltinFunction(func) => func.name(db),
        }
    }

    pub fn params(&self, db: &dyn Db) -> Vec<(Param, Type)> {
        self.ty(db).params(db)
    }

    pub fn ty(&self, db: &dyn Db) -> Type {
        match self.0 {
            FunctionInner::HirDef(func) => TyKind::Function(func).intern(),
            FunctionInner::IntrinsicFunction(func) => {
                TyKind::IntrinsicFunction(func, Substitution::new_identity(func.num_vars(db)))
                    .intern()
            }
            FunctionInner::BuiltinFunction(func) => TyKind::BuiltinFunction(func).intern(),
        }
        .into()
    }

    pub fn ret_ty(&self, db: &dyn Db) -> Type {
        self.ty(db)
            .ty
            .ret_ty(db)
            .expect("expected return type")
            .into()
    }

    pub fn doc(&self, db: &dyn Db) -> Option<String> {
        match self.0 {
            FunctionInner::HirDef(func) => func.doc(db).map(|doc| doc.to_string()),
            FunctionInner::BuiltinFunction(func) => Some(func.doc(db).clone()),
            FunctionInner::IntrinsicFunction(func) => Some(func.doc(db).clone()),
        }
    }

    pub fn is_user_defined(&self) -> bool {
        matches!(self.0, FunctionInner::HirDef(_))
    }
}

impl From<HirDefFunction> for Function {
    fn from(func: HirDefFunction) -> Self {
        Self(FunctionInner::HirDef(func))
    }
}

impl From<IntrinsicFunction> for Function {
    fn from(func: IntrinsicFunction) -> Self {
        Self(FunctionInner::IntrinsicFunction(func))
    }
}

impl From<BuiltinFunction> for Function {
    fn from(func: BuiltinFunction) -> Self {
        Self(FunctionInner::BuiltinFunction(func))
    }
}

enum FunctionInner {
    HirDef(HirDefFunction),
    IntrinsicFunction(IntrinsicFunction),
    BuiltinFunction(BuiltinFunction),
}
