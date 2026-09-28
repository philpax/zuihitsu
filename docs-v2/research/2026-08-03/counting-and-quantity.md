# Lane: counting, quantity, and context

**Status: verified against primary sources, with one gap.** Written 2026-08-03, after the design chapters, in response to a question the earlier lanes never asked.

## Why this lane exists

None of the seven original lanes covered how to represent a count or a measure. The fact-shape lane mined the plurals literature for event semantics, citing Landman's *Events and plurality*, but only for how participants attach to an event, never for how many of something there are.

[The corpus study](modelling-study.md) surfaced the gap empirically: word counts, elapsed durations, ratings, and frequencies all sat in prose with their units, uninterpretable to any query. The count value type in [`statements.md`](../../statements.md) was then specified from first principles, with no survey behind it. This lane is that survey, run late.

It also answers a second question that turned out to be connected: how other systems handle the referential layering that the frame addresses.

## How the families represent counts

### OWL 2: cardinality as a class expression

Cardinality is expressed as a class, not as a value. `ObjectMinCardinality`, `ObjectMaxCardinality`, and `ObjectExactCardinality` each take a number and a property, and the **qualified** forms add a class expression restricting what is counted.

Asserting one of these of a named individual **is** legal, contrary to a first reading. The primer gives it directly:

```
ClassAssertion( ObjectExactCardinality( 5 :hasChild ) :John )
```

So "Vertas has five ice creams" is expressible in OWL 2 as membership of a qualified cardinality class.

Two properties make it awkward rather than wrong for our purposes.

**No unique name assumption.** The primer is explicit: "OWL does not make the assumption that different names are names for different individuals." Counting distinct fillers therefore requires stating inequality with `DifferentIndividuals`, and the primer notes this is essential for cardinality reasoning to function correctly. Five mentions are not five things until you say so.

**Open world.** "If some fact is not present in a database, it is usually considered false (the so-called closed-world assumption) whereas in the case of an OWL 2 document it may simply be missing (but possibly true)." For a personal agent that must answer what it actually knows, this is the wrong default, and it interacts badly with exact counts.

**No temporal scoping at all.** A count that changes over time requires reifying the assertion as a fluent, which is the reification tax we already pay elsewhere and would rather not pay twice.

### Wikidata: quantity as a datatype with uncertainty

The `quantity` datatype carries four fields:

| | |
|---|---|
| `amount` | the primary value |
| `unit` | a unit item, which may be empty for dimensionless values such as a count |
| `lowerBound` | optional lower uncertainty bound |
| `upperBound` | optional upper uncertainty bound |

Uncertainty is expressed through the bounds, entered as a nominal value plus or minus a tolerance. Amount and both bounds are stored as strings, capped at 127 characters. Time scoping rides on statement qualifiers rather than on the value.

**This is a strictly better value type than the one the design specified.** A count over a kind, as drafted, had no way to distinguish "about a hundred thousand words" from "exactly a hundred thousand words". The observed corpus contains both shapes and several that are explicitly approximate.

### Conceptual graphs: plural referents, and the distinction we punted

Sowa's notation carries counts natively inside the concept box. `[Cat: {*}@3]` denotes three cats, unnamed. `[Dog: {Lucky, Macula}]` denotes a named set.

More importantly, CGs distinguish **collective** from **distributive** plurals explicitly in the notation: `Col{*}@2` means every cat has one set containing two ears, while `Dist{*}@2` means the cat has two ears as separate parts.

This is the distinction [`statements.md`](../../statements.md) declines to make, on the grounds that it has not been worth the machinery. That remains a defensible call, but the design should say it is declining a solved problem rather than an open one. The CG community's own assessment is that simple versions are implemented and the difficulty lies in generalising them across the full range of natural-language variation.

### Cyc: full quantification, and microtheories

Cyc has first-order quantification and reified sets, so a cardinality claim about a set is direct. The knowledge base is divided into hundreds of microtheories, each internally consistent while contradicting others, and each a first-class object in the ontology. Truth in CycL is context-relative.

**The microtheory example the literature reaches for is exactly our frame problem.** Cyc asserts in `#$TheSimpsonsMt` that Bart is a male fourth-grader, and in `#$RealWorldDataMt` that Bart is a cartoon character. The system can enter the fictional context when appropriate while knowing that cartoon characters are not people.

That is the persona-against-actual distinction, solved generally, decades before we met it in a corpus of persona agents. Our [frame](../../statements.md) is a three-valued, closed, non-nestable microtheory.

### Property graphs and TypeDB

Property graphs put a `count` property on a node or edge, with no semantics and no reasoning. Pragmatic, and roughly where we are with better bookkeeping around it.

**TypeDB's cardinality annotations could not be verified.** The documentation returned HTTP 403. The claim that TypeQL carries schema-level cardinality annotations is left unverified rather than asserted.

## What this changes in the design

**Take Wikidata's quantity shape wholesale.** A count or measure carries an amount, an optional unit, and optional bounds. This costs two nullable fields and buys the approximate-against-exact distinction that the corpus demonstrably needs.

**Name Cyc microtheories as the frame's ancestor.** The frame is not an invention. It is a deliberate simplification: closed where microtheories are open, three-valued where they are a lattice, and checkable by a critic where a general context logic is not. Presenting it as new would misrepresent both its novelty and its limits.

**State that collective and distributive is a declined distinction, not an unknown one.** Conceptual graphs solved it. We are choosing not to pay for it, and the escape route if plurals become load-bearing is CG notation and the lattice-theoretic plurals literature behind it.

**Do not adopt a formalism.** Each system that beats us on counting loses on the axes that dominate real recorded content. OWL has no temporal scoping and the wrong world assumption. Cyc has the curation economics already recorded in the graveyard lessons. Conceptual graphs are a representation without a storage, provenance, or revision story. The counting question is a small corner of the task.

## Confidence

| Claim | Status |
|---|---|
| OWL 2 qualified cardinality, and `ClassAssertion` of a cardinality expression | Verified against the OWL 2 primer |
| OWL 2 lacks a unique name assumption; `DifferentIndividuals` is needed for counting | Verified, quoted |
| OWL 2 is open-world by design | Verified, quoted |
| Wikidata quantity carries amount, unit, lowerBound, upperBound | Verified |
| CG plural referent notation `{*}@n`, and `Col`/`Dist` | Verified |
| Cyc microtheories are context-relative and used for fictional content | Verified, with the Simpsons example |
| TypeDB cardinality annotations | **Unverified.** Documentation unreachable |
| That no better whole-system fit exists for our task | Judgement, not a finding |
