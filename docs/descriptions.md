# Described definitions

How the `host` module binds a definition service from the description its C library answers, so a Rust plugin calls a
library that ships no Rust source: the load, the bound object, the values it takes and answers, handles, carried
definitions, enums and which thread a call runs on. What a description is, and the check that holds it, are the
standard's `docs/descriptions.md`. The code is in [src/host/description/](source/src/host/description/README.md). When
the binary closes and how a holder lets go is the same for a bound definition as for any other, in [host.md](host.md).

## The load

- **The check is the caller's.** The host never loads the standard, so it runs the `DescriptionCheck` its caller hands
  it in `HostOptions`, as `standard::StandardDescriptions` provides it, before it reads anything else of a
  description: `weivel_standard_description_check`, then `weivel_standard_description_answers` with the definition the
  service's symbol answered, so a description of another definition refuses the load even when both share one struct.
  The kinds each role takes, the shape of a refusal and a release, and where the context lies are the check's rules,
  which the binding does not repeat.
- **The layout is the platform's.** The description is then read once into Rust values, and every struct it names is
  laid out by this platform's C rules from its members in order, a definition's operations among its members by
  offset: every member's offset and size, and every struct's size, must be what the description states, or the load is
  refused as the caller's mistake naming the member. So is a kind above the last the binding knows, an element a
  pointer or an array cannot hold, an operation of more than 7 parameters, one that takes no context first, a
  description listing a `weivel_status` that is not `core.h`'s, a name in the description that is not UTF-8, and a
  definition whose struct ends before its first shape.
- **Plans are built at the load.** Each operation's steps, its answer and refusal blocks, its function, the
  definition's context and every release are made once; a call looks up only its operation by name. The described
  definition is registered awaiting releases as its `BoundDefinition`, whose `address` is the very address its
  definition's symbol answered, which is also the address it crosses the plugin seam as.

## Calls and values

`BoundDefinition::call(operation, inputs)` takes `Inputs`, a map keyed by input parameter name, over one `Value` enum:
`U32` (an enum's value included), `U64`, `I64`, `F64`, `Size`, `Text` (none for NULL), `Texts`, `Bytes`, `Array`,
`Struct` (`Members`, a struct's members by name in the order they were put, an answer's in the description's
order, its names shared with the description rather than copied), `Tree` (the standard's value tree), `Handle` and `Definition`. A
pointer is what it points at, a counted array, texts or bytes carry their own count, which the binding writes, and a
left-out input or member is its kind's zero: 0, NULL, an empty array or a value of kind NONE. A leading `size`, every
count, the context and storage are the binding's: naming one, a name the operation or struct does not take, a value of
another kind than its parameter's, a text holding a NUL, a number in a value tree that is not finite, a key twice in a
record, or structs nested deeper than `WEIVEL_STANDARD_VALUE_DEPTH_LIMIT` is the caller's mistake, and nothing is
called. Every input is written into memory of the call's own, which lives until the call answers: cut, with the
answer and refusal blocks, from an arena the call's thread keeps and empties again, so a call allocates only what its
answer is made of. Where a wrong value lies is spelled out only when it is wrong. `tests/allocations` pins how many
allocations a `reflect` and a `negate` make, so a call that grows slower by allocating fails a test.

`Tree::to_json` writes a value tree as compact JSON for any caller that hands one on as text: each record's members in
the order it holds them, a number in the shortest form that reads back as itself, and a text with its quote, backslash
and control characters escaped. A number that is not finite has no JSON form and answers `NonFiniteNumber`, the first
one met; the conformance suite writes each vector's files through it.

A call answers `Ok(DescribedAnswer::Answered(answers))` on OK, each answer by name, a NULL pointer, a NULL handle, a
NULL definition and a value of kind NONE left out, `Answers::value` reading an operation's one answer; and
`Ok(DescribedAnswer::None)` on NONE. Anything else is a `DescribedCallError` carrying the `operation`, its `status` as
a `CallStatus`, and `detail` and `fix`: `Returned` with the provider's own `weivel_status`, its refusal's texts for
REFUSED; `CallersMistake` for what was refused before anything was called, an operation the definition has none of or
its provider does not offer (its pointer past the `size` the definition answered, or NULL there) included;
`Unloaded` once the library's binary has closed; and `ProviderFault` for an answer the binding cannot read, as one
nesting deeper than the limit, a record holding a key twice or a text that is not UTF-8, which is never replaced; the
answer is released all the same. A struct lies one level below the one holding it by value or reaching it through a
pointer or an array, and both walks keep their own stack rather than recursing, so the limit is reached on any
thread's stack. Every answer and the
refusal are released, once their values are copied out, through the release the description names for each, but an
answer carrying a definition; an answer naming no release, as a text the provider keeps, is released by no one.

## Handles and carried definitions

- **A handle is a value whose drop releases it once.** A HANDLE answer, or a HANDLE member of an answer, is a
  `DescribedHandle`, freed through the release its description names by `DescribedHandle::release`, by the drop of its
  last clone, or by its library's unload, which releases every handle still held while the binary is still mapped; any
  later release does nothing. Handed back as an input it must be live, of the same library and of the type the input
  takes, and the call uses it until it returns: a release meanwhile, from another thread or by the unload, marks it
  released at once and frees it only once the last call using it has returned.
- **A definition an answer carries is bound too.** A DEFINITION answer or member is a `BoundDefinition`, bound from the
  description nested in its carrier's, whose layouts and plans were built at the load. It lives, with every handle and
  answer got through it, as long as the answer that carried it: `Answers::release` ends that answer, so every later
  call through the definition is the caller's mistake, and once the calls running in it have returned, whatever was
  got through it is released, then the answer through the release its description names. The drop of the answer and
  of every clone of the definitions it carried ends it too, and so does the library's unload.
- **Binary lifetime.** A holding outlives the unload, so a bound definition stays callable while its holder holds it,
  and answers `Unloaded` once the binary has closed; each call holds the binary mapped until it returns. The unload
  shuts the library's holdings as it releases them, so a handle or a carried definition that a call still running
  answers afterwards is released as it is answered.

## Enums and blocking

- **Enums by name.** `BoundDefinition::enums` answers the `DescribedEnums` of its description, each enum by the name its
  header spells holding its entries' numbers by name, with `weivel_status` beside them as `core.h` spells it. A plugin
  reads them through its context, `Context::enums_of(object)`, which answers none for an object no description bound,
  and the same enums for a definition reached across the plugin seam, which the plugin's side binds again from the
  description the host hands beside it, and `Context::require_enums(enums, names, of)` answers exactly
  the enums and entries it names, the caller's mistake naming the first one the description lacks, so a name it does
  not carry stops its reader rather than reading as missing in every comparison.
- **Blocking is reported, not acted on.** `BoundDefinition::blocking_operations` names the operations the description
  marks `blocking`, in the struct's order. Every operation runs on the calling thread, those included, as Java's do: a
  Rust caller that must not wait is async and hands such a call to its own runtime's blocking pool.

## Binding with no library

`BoundDefinition::bind(definition, description)` binds a definition its caller got some other way, as a library's is
bound. It is `unsafe`, since the caller promises both pointers outlive the bound definition and that the standard's
check passed the description; no library stands behind it, so nothing is released at an unload, and every handle and
carried definition lives until its own release or drop.

## The hand layouts

The binding declares every struct of `description.h` and `value.h` by hand, and the standard's
`weivel_standard_refusal` beside them; `standard::declarations::standard_struct_layouts` lists each one's fields as
this compiler lays them out, and the layout test holds them to what the standard's `layouts` program prints. Each call
is made through a function pointer typed at the call from its parameters' kinds, every integer and pointer in a 64-bit
word and a `double` as itself, so the binding refuses to bind on a platform whose pointers are not 64 bits.
