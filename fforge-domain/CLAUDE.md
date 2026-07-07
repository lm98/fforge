# fforge-domain

Layer 1 of the fforge workspace: pure domain model. No I/O, no RNG, no wall-clock, no
dependency on any layer above this one. Every type here is a data definition or a pure
function — the crate is a library; the binary lives in `fforge-game`.

## Module map

| Module | Owns |
|---|---|
| `attributes` | `Attribute` enum (25 variants), `Attributes` dense array, `Rating` type, `DevCategory` |
| `role` | `Role` enum (8 variants), `RoleWeights`, `ROLE_WEIGHTS` static table |
| `ability` | `current_ability()`, `best_role()` — CA semantics |
| `character` | `Character` (hidden attributes: potential, determination, professionalism, consistency, injury_proneness, leadership) |
| `entities` | ID newtypes, `Player`, `Staff`, `Club`, `Competition`, `Fixture`, `World` |
| `date` | `GameDate` — flat 365-day sim calendar, no leap years, no wall clock |
| `formation` | `FormationDef`, `Lineup`, `FORMATIONS` — 4 hardcoded formations |

`lib.rs` re-exports everything public; consumers import from the crate root.
