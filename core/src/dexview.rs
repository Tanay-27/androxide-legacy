//! Per-DEX views for `androguard_rs.dex.DEX`: what androguard 4.1.3's DEX
//! objects (`ClassDefItem`, `EncodedMethod`, `EncodedField`) hold, read
//! from one class_def at a time.

use anyhow::Result;
use rustc_hash::FxHashMap;

use crate::classes::{Value, class_info, field_flags, method_flags};
use crate::dex::{Dex, Member};
use crate::render::AgResolver;

/// An EncodedMethod as androguard has it after `load()`.
pub struct MethodRow {
    /// `get_class_name()`: its method_id's class.
    pub class_name: String,
    pub name: String,
    /// `get_descriptor()`: "(P1 P2)R".
    pub descriptor: String,
    pub access_flags: u64,
    pub flags_string: String,
    /// `get_code_off()`, as read.
    pub code_off: u64,
    /// `get_code()` is not None.
    pub has_code: bool,
    /// `get_code().get_registers_size()`.
    pub registers: Option<u16>,
    pub method_idx: u32,
}

/// An EncodedField as androguard has it.
pub struct FieldRow {
    pub class_name: String,
    pub name: String,
    pub ty: String,
    pub access_flags: u64,
    pub flags_string: String,
    pub init: Option<Value>,
    pub field_idx: u32,
}

/// A class_def's own part: name, access flags string, superclass,
/// interfaces, class_data_off (0 without class_data).
pub struct ClassHeader {
    pub name: String,
    pub access_flags: String,
    pub superclass: String,
    pub interfaces: Vec<String>,
    pub class_data_off: u32,
}

/// A class_data's EncodedMethods (direct, then virtual) and EncodedFields
/// (static, then instance). Every class_def using a class_data gets the
/// same (androguard shares one ClassDataItem object, static values
/// included), so callers can keep them per class_data_off.
pub struct Members {
    pub methods: Vec<MethodRow>,
    pub fields: Vec<FieldRow>,
}

/// class_def `ci`'s header. Errors where androguard's `DEX()` would fail on
/// the class (see `class_info`).
pub fn class_header(dex: &Dex, ci: usize) -> Result<ClassHeader> {
    let cd = dex.class_def(ci)?;
    let info = class_info(dex, ci)?;
    Ok(ClassHeader {
        name: info.name,
        access_flags: info.access_flags,
        superclass: info.superclass,
        interfaces: info.interfaces,
        class_data_off: if dex.has_class_data(cd.class_data_off) { cd.class_data_off } else { 0 },
    })
}

/// The members of class_def `ci`'s class_data (empty without one).
pub fn class_members(dex: &Dex, ci: usize) -> Result<Members> {
    let info = class_info(dex, ci)?;
    let cd = dex.class_def(ci)?;
    let r = AgResolver { dex };
    let has_data = dex.has_class_data(cd.class_data_off);
    let mut methods = Vec::new();
    let mut fields = Vec::new();
    if has_data {
        let data = dex.class_data(cd.class_data_off)?;
        for (m, (name, desc, flags)) in data.direct_methods.iter().chain(&data.virtual_methods).zip(info.methods.iter()) {
            let class_name = match dex.method_id(m.idx) {
                Ok((c, _, _)) => r.type_name(c),
                Err(_) => "AG:IMI:invalid_class_name;".into(),
            };
            methods.push(MethodRow {
                class_name,
                name: name.clone(),
                descriptor: desc.clone(),
                access_flags: m.access_flags,
                flags_string: flags.clone(),
                code_off: m.code_off_raw,
                has_code: dex.has_code_item(m.code_off),
                registers: dex.registers_size(m.code_off),
                method_idx: m.idx,
            });
        }
        let members = data.static_fields.iter().chain(&data.instance_fields);
        for (m, (_, name, ty, init)) in members.zip(info.field_values()) {
            let class_name = match dex.field_id(m.idx) {
                Some((c, _, _)) => r.type_name(c),
                None => "AG:IFI:invalid_class_name;".into(),
            };
            fields.push(FieldRow {
                class_name,
                name: name.to_owned(),
                ty: ty.to_owned(),
                access_flags: m.access_flags,
                flags_string: field_flags(m.access_flags),
                init: init.cloned(),
                field_idx: m.idx,
            });
        }
    }
    debug_assert!(methods.iter().all(|m| m.flags_string == method_flags(m.access_flags)));
    Ok(Members { methods, fields })
}

/// `get_classes_names()` of one DEX: every class_def's name, in order.
pub fn class_names(dex: &Dex) -> Vec<String> {
    let r = AgResolver { dex };
    (0..dex.class_count())
        .map(|ci| dex.class_def(ci).map(|cd| r.type_name(cd.class_idx)).unwrap_or_default())
        .collect()
}

/// See `Apk.dex_find_member`.
pub fn find_member(dex: &Dex, key: &[u8], fields: bool) -> Option<(usize, usize)> {
    let r = AgResolver { dex };
    let mut found = None;
    let mut per_data: FxHashMap<u32, Option<usize>> = FxHashMap::default();
    for ci in 0..dex.class_count() {
        let Ok(cd) = dex.class_def(ci) else { continue };
        if !dex.has_class_data(cd.class_data_off) {
            continue;
        }
        let hit = *per_data.entry(cd.class_data_off).or_insert_with(|| {
            let data = dex.class_data(cd.class_data_off).ok()?;
            let text = |m: &Member| -> String {
                if fields {
                    match dex.field_id(m.idx) {
                        Some((c, t, n)) => format!("{}{}{}", r.type_name(c), r.string(n), r.type_name(t)),
                        None => "AG:IFI:invalid_class_name;AG:IFI:invalid_name(AG:IFI:invalid_type)".into(),
                    }
                } else {
                    let class = match dex.method_id(m.idx) {
                        Ok((c, _, _)) => r.type_name(c),
                        Err(_) => "AG:IMI:invalid_class_name;".into(),
                    };
                    let (name, desc) = r.method_name_and_desc(m.idx);
                    format!("{class}{name}{desc}")
                }
            };
            let list: Vec<&Member> = if fields {
                data.static_fields.iter().chain(&data.instance_fields).collect()
            } else {
                data.direct_methods.iter().chain(&data.virtual_methods).collect()
            };
            list.iter().rposition(|m| crate::render::wtf8(&text(m)).as_ref() == key)
        });
        if let Some(pos) = hit {
            found = Some((ci, pos));
        }
    }
    found
}

/// See `Apk.dex_field_names`.
pub fn field_names(dex: &Dex) -> Vec<(usize, usize, String)> {
    let r = AgResolver { dex };
    let mut out = Vec::new();
    for ci in 0..dex.class_count() {
        let Ok(cd) = dex.class_def(ci) else { continue };
        if !dex.has_class_data(cd.class_data_off) {
            continue;
        }
        let Ok(data) = dex.class_data(cd.class_data_off) else { continue };
        for (pos, m) in data.static_fields.iter().chain(&data.instance_fields).enumerate() {
            let name = match dex.field_id(m.idx) {
                Some((_, _, n)) => r.string(n),
                None => "AG:IFI:invalid_name".into(),
            };
            out.push((ci, pos, name));
        }
    }
    out
}
