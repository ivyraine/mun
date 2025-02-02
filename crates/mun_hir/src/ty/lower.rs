//! Methods for lower the HIR to types.

pub(crate) use self::diagnostics::LowerDiagnostic;
use crate::resolve::{HasResolver, TypeNs};
use crate::ty::{Substitution, TyKind};
use crate::{
    arena::map::ArenaMap,
    code_model::StructKind,
    diagnostics::DiagnosticSink,
    name_resolution::Namespace,
    primitive_type::PrimitiveType,
    resolve::Resolver,
    ty::{FnSig, Ty},
    type_ref::{LocalTypeRefId, TypeRef, TypeRefMap, TypeRefSourceMap},
    FileId, Function, HirDatabase, ModuleDef, Path, Struct, TypeAlias,
};
use crate::{HasVisibility, Visibility};
use std::{ops::Index, sync::Arc};

/// A struct which holds resolved type references to `Ty`s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LowerTyMap {
    pub(crate) type_ref_to_type: ArenaMap<LocalTypeRefId, Ty>,
    pub(crate) diagnostics: Vec<LowerDiagnostic>,

    unknown_ty: Ty,
}

impl Default for LowerTyMap {
    fn default() -> Self {
        LowerTyMap {
            type_ref_to_type: Default::default(),
            diagnostics: vec![],
            unknown_ty: TyKind::Unknown.intern(),
        }
    }
}

impl Index<LocalTypeRefId> for LowerTyMap {
    type Output = Ty;
    fn index(&self, expr: LocalTypeRefId) -> &Ty {
        self.type_ref_to_type.get(expr).unwrap_or(&self.unknown_ty)
    }
}

impl LowerTyMap {
    /// Adds all the `LowerDiagnostic`s of the result to the `DiagnosticSink`.
    pub(crate) fn add_diagnostics(
        &self,
        db: &dyn HirDatabase,
        file_id: FileId,
        source_map: &TypeRefSourceMap,
        sink: &mut DiagnosticSink,
    ) {
        self.diagnostics
            .iter()
            .for_each(|it| it.add_to(db, file_id, source_map, sink))
    }
}

impl Ty {
    /// Tries to lower a HIR type reference to an actual resolved type. Besides the type also
    /// returns an diagnostics that where encountered along the way.
    pub(crate) fn from_hir(
        db: &dyn HirDatabase,
        resolver: &Resolver,
        type_ref_map: &TypeRefMap,
        type_ref: LocalTypeRefId,
    ) -> (Ty, Vec<diagnostics::LowerDiagnostic>) {
        let mut diagnostics = Vec::new();
        let ty =
            Ty::from_hir_with_diagnostics(db, resolver, type_ref_map, &mut diagnostics, type_ref);
        (ty, diagnostics)
    }

    /// Tries to lower a HIR type reference to an actual resolved type. Takes a mutable reference
    /// to a `Vec` which will hold any diagnostics encountered a long the way.
    fn from_hir_with_diagnostics(
        db: &dyn HirDatabase,
        resolver: &Resolver,
        type_ref_map: &TypeRefMap,
        diagnostics: &mut Vec<LowerDiagnostic>,
        type_ref: LocalTypeRefId,
    ) -> Ty {
        let res = match &type_ref_map[type_ref] {
            TypeRef::Path(path) => Ty::from_path(db, resolver, type_ref, path, diagnostics),
            TypeRef::Error => Some((TyKind::Unknown.intern(), false)),
            TypeRef::Tuple(inner) => {
                let inner_tys = inner.iter().map(|tr| {
                    Self::from_hir_with_diagnostics(db, resolver, type_ref_map, diagnostics, *tr)
                });
                Some((
                    TyKind::Tuple(inner_tys.len(), inner_tys.collect()).intern(),
                    false,
                ))
            }
            TypeRef::Never => Some((TyKind::Never.intern(), false)),
            TypeRef::Array(inner) => {
                let inner = Self::from_hir_with_diagnostics(
                    db,
                    resolver,
                    type_ref_map,
                    diagnostics,
                    *inner,
                );
                Some((TyKind::Array(inner).intern(), false))
            }
        };
        if let Some((ty, is_cyclic)) = res {
            if is_cyclic {
                diagnostics.push(LowerDiagnostic::CyclicType { id: type_ref })
            }
            ty
        } else {
            diagnostics.push(LowerDiagnostic::UnresolvedType { id: type_ref });
            TyKind::Unknown.intern()
        }
    }

    /// Constructs a `Ty` from a path.
    fn from_path(
        db: &dyn HirDatabase,
        resolver: &Resolver,
        type_ref: LocalTypeRefId,
        path: &Path,
        diagnostics: &mut Vec<LowerDiagnostic>,
    ) -> Option<(Self, bool)> {
        // Find the type
        let (ty, vis) = resolver.resolve_path_as_type_fully(db.upcast(), path)?;

        // Get the definition and visibility
        let def = match ty {
            TypeNs::StructId(id) => TypableDef::Struct(id.into()),
            TypeNs::TypeAliasId(id) => TypableDef::TypeAlias(id.into()),
            TypeNs::PrimitiveType(id) => TypableDef::PrimitiveType(id),
        };

        // Get the current module and see if the type is visible from here
        if let Some(module) = resolver.module() {
            if !vis.is_visible_from(db, module) {
                diagnostics.push(LowerDiagnostic::TypeIsPrivate { id: type_ref })
            }
        }

        Some(db.type_for_def(def, Namespace::Types))
    }
}

/// Resolves all types in the specified `TypeRefMap`.
pub fn types_from_hir(
    db: &dyn HirDatabase,
    resolver: &Resolver,
    type_ref_map: &TypeRefMap,
) -> Arc<LowerTyMap> {
    let mut result = LowerTyMap::default();
    for (id, _) in type_ref_map.iter() {
        let ty =
            Ty::from_hir_with_diagnostics(db, resolver, type_ref_map, &mut result.diagnostics, id);
        result.type_ref_to_type.insert(id, ty);
    }
    Arc::new(result)
}

pub fn lower_struct_query(db: &dyn HirDatabase, s: Struct) -> Arc<LowerTyMap> {
    let data = s.data(db.upcast());
    types_from_hir(db, &s.id.resolver(db.upcast()), data.type_ref_map())
}

pub fn lower_type_alias_query(db: &dyn HirDatabase, t: TypeAlias) -> Arc<LowerTyMap> {
    let data = t.data(db.upcast());
    types_from_hir(db, &t.id.resolver(db.upcast()), data.type_ref_map())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypableDef {
    Function(Function),
    PrimitiveType(PrimitiveType),
    Struct(Struct),
    TypeAlias(TypeAlias),
}

impl From<Function> for TypableDef {
    fn from(f: Function) -> Self {
        TypableDef::Function(f)
    }
}

impl From<PrimitiveType> for TypableDef {
    fn from(f: PrimitiveType) -> Self {
        TypableDef::PrimitiveType(f)
    }
}

impl From<Struct> for TypableDef {
    fn from(f: Struct) -> Self {
        TypableDef::Struct(f)
    }
}

impl From<ModuleDef> for Option<TypableDef> {
    fn from(d: ModuleDef) -> Self {
        match d {
            ModuleDef::Function(f) => Some(TypableDef::Function(f)),
            ModuleDef::PrimitiveType(t) => Some(TypableDef::PrimitiveType(t)),
            ModuleDef::Struct(t) => Some(TypableDef::Struct(t)),
            ModuleDef::TypeAlias(t) => Some(TypableDef::TypeAlias(t)),
            ModuleDef::Module(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CallableDef {
    Function(Function),
    Struct(Struct),
}
impl_froms!(CallableDef: Function, Struct);

impl CallableDef {
    pub fn is_function(self) -> bool {
        matches!(self, CallableDef::Function(_))
    }

    pub fn is_struct(self) -> bool {
        matches!(self, CallableDef::Struct(_))
    }
}

impl HasVisibility for CallableDef {
    fn visibility(&self, db: &dyn HirDatabase) -> Visibility {
        match self {
            CallableDef::Struct(strukt) => strukt.visibility(db),
            CallableDef::Function(function) => function.visibility(db),
        }
    }
}

/// Build the declared type of an item. This depends on the namespace; e.g. for
/// `struct Foo(usize)`, we have two types: The type of the struct itself, and
/// the constructor function `(usize) -> Foo` which lives in the values
/// namespace.
pub(crate) fn type_for_def(db: &dyn HirDatabase, def: TypableDef, ns: Namespace) -> (Ty, bool) {
    let ty = match (def, ns) {
        (TypableDef::Function(f), Namespace::Values) => type_for_fn(db, f),
        (TypableDef::PrimitiveType(t), Namespace::Types) => type_for_primitive(t),
        (TypableDef::Struct(s), Namespace::Values) => type_for_struct_constructor(db, s),
        (TypableDef::Struct(s), Namespace::Types) => type_for_struct(db, s),
        (TypableDef::TypeAlias(t), Namespace::Types) => type_for_type_alias(db, t),

        // 'error' cases:
        (TypableDef::Function(_), Namespace::Types) => TyKind::Unknown.intern(),
        (TypableDef::PrimitiveType(_), Namespace::Values) => TyKind::Unknown.intern(),
        (TypableDef::TypeAlias(_), Namespace::Values) => TyKind::Unknown.intern(),
    };
    (ty, false)
}

/// Recover with an unknown type when a cycle is detected in the salsa database.
pub(crate) fn type_for_cycle_recover(
    _db: &dyn HirDatabase,
    _cycle: &[String],
    _def: &TypableDef,
    _ns: &Namespace,
) -> (Ty, bool) {
    (TyKind::Unknown.intern(), true)
}

/// Build the declared type of a static.
fn type_for_primitive(def: PrimitiveType) -> Ty {
    match def {
        PrimitiveType::Float(f) => TyKind::Float(f.into()),
        PrimitiveType::Int(i) => TyKind::Int(i.into()),
        PrimitiveType::Bool => TyKind::Bool,
    }
    .intern()
}

/// Build the declared type of a function. This should not need to look at the
/// function body.
fn type_for_fn(_db: &dyn HirDatabase, def: Function) -> Ty {
    TyKind::FnDef(def.into(), Substitution::empty()).intern()
}

pub(crate) fn callable_item_sig(db: &dyn HirDatabase, def: CallableDef) -> FnSig {
    match def {
        CallableDef::Function(f) => fn_sig_for_fn(db, f),
        CallableDef::Struct(s) => fn_sig_for_struct_constructor(db, s),
    }
}

pub(crate) fn fn_sig_for_fn(db: &dyn HirDatabase, def: Function) -> FnSig {
    let data = def.data(db.upcast());
    let resolver = def.id.resolver(db.upcast());
    let params = data
        .params()
        .iter()
        .map(|tr| Ty::from_hir(db, &resolver, data.type_ref_map(), *tr).0)
        .collect::<Vec<_>>();
    let ret = Ty::from_hir(db, &resolver, data.type_ref_map(), *data.ret_type()).0;
    FnSig::from_params_and_return(params, ret)
}

pub(crate) fn fn_sig_for_struct_constructor(db: &dyn HirDatabase, def: Struct) -> FnSig {
    let data = def.data(db.upcast());
    let resolver = def.id.resolver(db.upcast());
    let params = data
        .fields
        .iter()
        .map(|(_, field)| Ty::from_hir(db, &resolver, data.type_ref_map(), field.type_ref).0)
        .collect::<Vec<_>>();
    let ret = type_for_struct(db, def);
    FnSig::from_params_and_return(params, ret)
}

/// Build the type of a struct constructor.
fn type_for_struct_constructor(db: &dyn HirDatabase, def: Struct) -> Ty {
    let struct_data = db.struct_data(def.id);
    if struct_data.kind == StructKind::Tuple {
        TyKind::FnDef(def.into(), Substitution::empty()).intern()
    } else {
        type_for_struct(db, def)
    }
}

fn type_for_struct(_db: &dyn HirDatabase, def: Struct) -> Ty {
    TyKind::Struct(def).intern()
}

fn type_for_type_alias(db: &dyn HirDatabase, def: TypeAlias) -> Ty {
    let data = def.data(db.upcast());
    let resolver = def.id.resolver(db.upcast());
    let type_ref = def.type_ref(db);
    Ty::from_hir(db, &resolver, data.type_ref_map(), type_ref).0
}

pub mod diagnostics {
    use crate::diagnostics::{CyclicType, PrivateAccess, UnresolvedType};
    use crate::{
        diagnostics::DiagnosticSink,
        type_ref::{LocalTypeRefId, TypeRefSourceMap},
        FileId, HirDatabase,
    };

    #[derive(Debug, PartialEq, Eq, Clone)]
    pub(crate) enum LowerDiagnostic {
        UnresolvedType { id: LocalTypeRefId },
        TypeIsPrivate { id: LocalTypeRefId },
        CyclicType { id: LocalTypeRefId },
    }

    impl LowerDiagnostic {
        pub(crate) fn add_to(
            &self,
            _db: &dyn HirDatabase,
            file_id: FileId,
            source_map: &TypeRefSourceMap,
            sink: &mut DiagnosticSink,
        ) {
            match self {
                LowerDiagnostic::UnresolvedType { id } => sink.push(UnresolvedType {
                    file: file_id,
                    type_ref: source_map.type_ref_syntax(*id).unwrap(),
                }),
                LowerDiagnostic::CyclicType { id } => sink.push(CyclicType {
                    file: file_id,
                    type_ref: source_map.type_ref_syntax(*id).unwrap(),
                }),
                LowerDiagnostic::TypeIsPrivate { id } => sink.push(PrivateAccess {
                    file: file_id,
                    expr: source_map.type_ref_syntax(*id).unwrap().syntax_node_ptr(),
                }),
            }
        }
    }
}
