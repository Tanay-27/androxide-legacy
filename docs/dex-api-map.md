# DEX, `Analysis` and the per-DEX objects vs androguard 4.1.3

What `androguard_rs.core.dex` (`DEX`, `ClassDefItem`, `EncodedMethod`,
`EncodedField`, ...), `androguard_rs.core.analysis.analysis` (`Analysis`,
`ClassAnalysis`, `MethodAnalysis`) and `androguard_rs.misc.AnalyzeAPK`
do where androguard 4.1.3's do, including how loading fails.

## Module layout

`androguard_rs` has androguard 4.1.3's module layout, so code switches by
renaming `androguard` to `androguard_rs` in its imports, same paths
(`androguard_rs.core.dex` is the module `androguard_rs.dex`, the same
classes; `python/tests/test_layout.py`):

| androguard 4.1.3 | `androguard_rs` (same module as) |
|---|---|
| `androguard.core.apk` (`APK`, `FileNotPresent`; `from androguard.core import apk`) | `androguard_rs.core.apk` (`androguard_rs.apk`) |
| `androguard.core.axml` (`AXMLPrinter`, `ARSCParser`, `ARSCResTableConfig`) | `androguard_rs.core.axml` (`androguard_rs.axml`; `ARSCParser` is `androguard_rs.apk.ARSCParser`) |
| `androguard.core.dex` (`DEX`, `ClassDefItem`, `EncodedMethod`, `DalvikCode`, `DCode`, `Instruction*`) | `androguard_rs.core.dex` (`androguard_rs.dex`) |
| `androguard.core.analysis.analysis` (`Analysis`, `ClassAnalysis`, `MethodAnalysis`) | `androguard_rs.core.analysis.analysis` (`androguard_rs.analysis`) |
| `androguard.core.api_specific_resources` (`load_permissions`, `load_permission_mappings`) | `androguard_rs.core.api_specific_resources` (`androguard_rs.api_specific_resources`) |
| `androguard.core.androconf.load_api_specific_resource_module` | `androguard_rs.core.androconf` |
| `androguard.util.set_log` | `androguard_rs.util.set_log` (no-op) |
| `androguard.misc.AnalyzeAPK` | `androguard_rs.misc.AnalyzeAPK` |
| `androguard.decompiler` | `androguard_rs.decompiler`: import only, the decompiler is not provided (see RecursionError below) |

## Construction and load errors

The usual manual load runs unchanged on the drop-ins:

```python
from androguard_rs.core.apk import APK
from androguard_rs.core.dex import DEX
from androguard_rs.core.analysis.analysis import Analysis

a = APK(raw, raw=True)
vmx = Analysis()
for b in a.get_all_dex():
    vm = DEX(b)
    vmx.add(vm)
    vm.set_analysis(vmx)
vmx.create_xref()
```

- `DEX(bytes)`: raises what androguard's `DEX()` raises, at the same
  point (`core/src/agload.rs` steps through its parse over the raw bytes):
  every `HeaderItem` check (`ValueError("Not a DEX file, Header too
  small.")`, `ValueError("This is not a DEX file! Wrong endian tag:
  '0x…'")`, `NotImplementedError("Byte swapped endian tag
  encountered!")`, wrong magic, Adler-32 checksum, header size, too many
  type or proto ids), then the map list and every section it declares, in
  androguard's load order: `struct.error` for a short read,
  `ValueError('N is not a valid TypeMapItem')`, `KeyError(<TypeMapItem.…>)`
  for a missing section something resolves through, `KeyError(off)` for an
  undeclared type_list, `AttributeError` for a method_id whose proto is out
  of range ("'ProtoIdItemInvalid' object has no attribute …") or a class
  whose static values aren't there ("'NoneType' object has no attribute
  'get_value'"), a moved HEADER_ITEM's header checks, hidden-API flag enum
  errors, `RecursionError` for nested encoded values (see below). The
  exception is built so that type, `args` and `str()` are androguard's
  (`androguard_rs._agerr`; `TypeMapItem` and
  `HiddenApiClassDataItem.RestrictionApiFlag`/`DomapiApiFlag` are in
  `androguard_rs.dex`). The bytes are kept.
- `Analysis()` / `add(vm)`: as androguard's `add()`, appends the DEX and
  raises what it raises for it: `AttributeError("'DEX' object has no
  attribute 'classes'")` without a map list, `IndexError` for a class with
  no hidden-API flags, `KeyError(<TypeMapItem.METHOD_ID_ITEM: 5>)` without
  method_ids, `InvalidInstruction` (same args and `str()`) for the first
  method it can't disassemble, `struct.error` for code cut to an odd length
  by the end of the file, `IndexError` for a try whose handler offset
  matches no handler (`determineException`), in androguard's order.
- `create_xref()`: raises what androguard's raises walking the
  instructions (`IndexError('string index out of range')` for a
  const-class/new-instance of, or an invoke on, a type whose name is empty
  once `[` are stripped; `KeyError` for a field/string reference without
  that section), then builds one native index (`androguard_rs.Apk`) over
  every added DEX, in order, binds each DEX object to it and drops the
  bytes. A second call returns, as androguard's does.
- `vm.set_analysis(vmx)`: kept, unused.

`androguard_rs.misc.AnalyzeAPK` gives the same objects (`d` is the list of
`DEX`, bound to the index) and raises at load what androguard's raises, in
its order: `DEX()` then `add()` for each DEX file in turn, `create_xref()`
last (`python/tests/test_strict.py`; `tolerant=True` loads anyway).

The id tables are read from the map list's sections, and a string_id whose
data the map doesn't declare is `AG:IS: invalid string`, as in androguard;
code cut short by the end of the file is read as far as it goes.

### RecursionError

androguard parses nested encoded values (arrays in arrays, annotations in
annotations) recursively, so a deep enough nesting raises
`RecursionError('maximum recursion depth exceeded')` from Python's
recursion limit, or, when that is high, from CPython 3.12's separate C
recursion limit (each nested instantiation counts). Where depends on the
caller's stack: `DEX()` and `AnalyzeAPK` measure what is left (Python
frames above the caller, and C units by nesting instantiations until
RecursionError, which takes about 2 ms) and the native walk counts each
frame and unit androguard's code would use, so the error comes at the same
nesting (`test_strict_parity.py::test_recursion_depth`: several caller
depths, both paths, arrays and annotations; the one assumption is that
loguru has no handler on that path, as androguard's own `set_log` above
DEBUG leaves it).

Importing `androguard.misc` or `androguard.decompiler` raises the recursion
limit to 5000 (`sys.setrecursionlimit` in
`androguard/decompiler/__init__.py`), and so does importing
`androguard_rs.misc` or `androguard_rs.decompiler` (which provides nothing
else; any other name from it raises `NotImplementedError`). Code that
drops its decompiler use should keep `import androguard_rs.decompiler` to
keep that limit.

### The one intentional difference

A string_data_item running to the end of the file without a NUL makes
androguard's `read_null_terminated_string` loop forever (its list of empty
reads grows until memory runs out). androguard-rs raises `MemoryError()` at
once.

## Per-DEX objects

| androguard API | here (`androguard_rs.dex`) |
|---|---|
| `DEX.get_classes()` | a list of `ClassDefItem`: `get_name()`, `get_superclassname()`, `get_interfaces()`, `get_access_flags_string()`, `get_methods()`, `get_fields()`, `str()`/`repr()` (`<dvm.ClassDefItem super->name>`); unknown attributes are `AttributeError` |
| `DEX.get_class(name)` | the first class_def of that name in this DEX, or None |
| `DEX.get_encoded_methods()` | every class_def's methods in order; a class_data shared by several class_defs is listed once per class_def (the same objects) |
| `get_encoded_method_descriptor(cls, name, desc)`, `get_encoded_field_descriptor(cls, name, desc)` | androguard's lookup: `class + name + descriptor` concatenated (descriptor with spaces), last match in class_def order; done natively, nothing cached |
| `get_encoded_field(regex)` | `re.match` on every field's name, in `get_encoded_fields()` order |
| `EncodedMethod.get_information()`, `get_access_flags_string()` | androguard 4.1.3's code verbatim (`get_type()` too), with `registers_size` read natively: `{}` without code, `"registers"` `(0, nb - len(params) - 1)`, `"params"` only with parameters (registers can be negative, as in androguard) |
| `EncodedMethod` attributes | `class_name` (its method_id's class), `name`, `proto`, `access_flags`, `code_off`, `method_idx`, `code`, `CM` (`CM.vm` is the method's DEX object) |
| `str(EncodedMethod)` | `cls->name(desc) [access_flags=…] @ 0x<code_off>` |
| `EncodedField.get_init_value().get_value()` | typed as androguard's (int, str, bool, None, lists for field/method references; arrays and annotations are objects whose `str()` is androguard's with the address as `0x0`); `None` without a value |

A class defined in two DEX files is in each DEX object with that DEX's
class_data (`test_dexobj.py::test_dup_class_per_dex`).

Memory: nothing is materialized per APK in Python. A DEX object's
`get_classes()` is a list of light `ClassDefItem`s (name only); a class's
methods and fields are fetched when asked and kept in a per-DEX LRU (512
classes), instructions in another (256 methods); `get_encoded_methods()` is
a lazy view (iterable, `len()`, indexable, `+`) made as it is walked.

## `Analysis.find_methods`, `MethodAnalysis`

- `find_methods(classname, methodname, descriptor, accessflags,
  no_external)`: every class in `dx.classes` order (external ones too), its
  `get_methods()`, matched with Python's `re.match` on the native rows'
  name, descriptor (with spaces) and flags (`""` for an ExternalMethod,
  which `no_external` drops), as androguard; unescaped class names,
  lookaheads and space-separated descriptors work as there.
- `MethodAnalysis.method`; `repr()`/`str()` is androguard's
  `<analysis.MethodAnalysis {method}>`; `get_method().get_instruction(idx,
  off)` as androguard's; `get_vm_class()` gives `super->name` (internal) or
  `<analysis.ExternalClass name>` (external) as `str()`.
- `get_xref_from()`/`get_xref_to()` return a `set` of `(ClassAnalysis,
  MethodAnalysis, offset)`.

Order: androguard returns a `set` of tuples hashed by object identity, so
its iteration order is the memory-address order of that run: arbitrary
and different from run to run. Here it is a `set` subclass (set
operations, `in`, `len`) that iterates in a fixed order, the index's (one
of androguard's possible orders, the same every run). Whatever a caller
derives from iteration order (`next(iter(...))`, last-wins dicts) is
deterministic here and run-dependent in androguard; every such result is
one androguard can produce. The tests compare these as sorted lists.

Identity: androguard has one object per method; here objects are made per
call and compare by what they stand for (`MethodAnalysis` by its method,
`EncodedMethod`/`EncodedField` by their class_data entry, `ExternalMethod`
by class/name/descriptor), so sets and dicts of them behave the same.

Unknown attributes: an attribute androguard's object doesn't have either
raises `AttributeError` with androguard's message; one it has that isn't
implemented raises `NotImplementedError` and is counted in `GAPS`.

## Tests

- `test_dexobj.py` (no androguard; goldens `dexobj_goldens.py` from
  `scripts/gen_dexobj_goldens.py`): every view above from `AnalyzeAPK`,
  from the manual load and from the facade, on `dexobj_cases.apk()`
  (TLS-related classes, Intent sources and sinks, typed static values, a
  class in both DEX files, two class_defs sharing a class_data, a
  per-method instruction walk); identity and AttributeError semantics.
- `test_dexobj_parity.py`: the same live, and `apk_views()` (every class,
  method and field of every DEX object, find_methods, xref callers,
  get_vm_class(), MethodAnalysis repr()s, the per-method walk) on each of
  `PARITY_APKS`.
- `test_strict.py` / `test_strict_parity.py`: load errors, `AnalyzeAPK`
  and the manual load (the header checks, the map list and its sections,
  add() and create_xref() failures, orders across DEX files; goldens from
  androguard 4.1.3 by `scripts/gen_dexobj_goldens.py`), the RecursionError
  depths live, and the case androguard never returns from.
- `test_layout.py`: the module paths above, and `core.androconf` against
  androguard's.
- `scripts/load_difffuzz.py`: mutates real DEX files and compares
  accept/reject and the exception (stage, type, `args`, `str()`) with
  androguard 4.1.3, one DEX or two per case; `--deep` also compares what
  the objects show for mutants both load.

## Gaps

- **Decompiler**: not provided (`androguard_rs.decompiler` is import-only).
- **Orders androguard derives from sets** (above): deterministic here.
- `str()` of an EncodedArray/EncodedAnnotation value: androguard prints a
  memory address; here `0x0`.
- `Instruction.cm` and `get_raw_string()` (`docs/instruction-api.md`).
- `ARSCParser.get_resolved_res_configs(rid, config=...)` with a config
  filter (`NotImplementedError`); without one, the result is androguard's,
  `ARSCResTableConfig` objects included (`docs/resources-api-map.md`).
- Crafted DEX files androguard loads but whose members can't be named
  (out-of-range method/type/string indices in class_data or ids,
  undecodable names, several classes with the same unresolvable name):
  androguard keeps one object per item, with placeholder names
  (`AG:IS: invalid string`, ...); the index keys methods and classes by
  name, so some of those members' xrefs and facade lookups differ (a
  facade lookup can raise NotImplementedError). Load errors are not
  affected. `scripts/load_difffuzz.py --deep` measures it; real APKs
  don't have such members.
- APK side (`APK`, `AXMLPrinter`, signing): `docs/apk-api-map.md`.
