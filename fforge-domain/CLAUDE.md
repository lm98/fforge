# fforge-domain

Layer 1: the pure domain model. Data definitions and pure functions only — no I/O, no RNG, no
wall clock, no dependency on any layer above. `docs/ATTRIBUTE_SCHEMA.md` is the specification;
this crate transcribes it.

## Module map

| Module | Owns |
|---|---|
| `attributes` | `Attribute` (25 variants), the dense `Attributes` array, `Rating`, `DevCategory` |
| `role` | `Role` (8 variants), `RoleWeights`, the `ROLE_WEIGHTS` table |
| `ability` | `current_ability()`, `best_role()` — the only path to a CA value |
| `character` | `Character`: the hidden attributes (potential, determination, professionalism, consistency, injury proneness, natural fitness, leadership) |
| `entities` | ID newtypes; `Player` (with `DevProfile`, `Contract`, `injured_until`, `retired`), `Staff`, `Club` (with `Finances`, `reputation`, `coaching_milli`), `Competition`, `Fixture`, `World`; `Money` |
| `date` | `GameDate` — a flat 365-day calendar: no leap years, no months, no wall clock |
| `formation` | `FormationDef`, the four `FORMATIONS`, `Lineup` (XI, `tactics`, `bench`, `sub_plan`), `BENCH_SIZE`, `MAX_SUBSTITUTIONS` |
| `tactics` | `Tactics` and its four ternary instructions (`Mentality`, `Tempo`, `Width`, `Pressing`); `neutral()` is all-`Balanced` |
| `substitution` | `SubRule { conditions, action }` with `SubCondition`, `SubAction`, `ScoreState` — the pre-committed in-match plan |

`lib.rs` re-exports everything public; consumers import from the crate root.

## Rules

- **This crate is closed to new features.** Changes are corrections, clarifications, and pure
  derived views over existing fields (`Player::age`, `Player::is_injured`). A new stored field
  needs a design note that sanctions it first, as `TRANSFER_MODEL.md` §3 and `MATCH_MODEL.md` §12
  did for the finance and Phase 2e types.
- **Do not add `Player.condition`.** `MATCH_MODEL.md` §12 originally listed it; condition is
  derived in `fforge-core` from recent appearances instead (§13).
- **`GameDate` stays monthless.** Human-readable dates are a presentation function in
  `fforge-game`.
- **Float-free where it is serialized into state.** `DevProfile` and `coaching_milli` are
  fixed-point, resolved once at worldgen, so replays are bit-stable across platforms.
- **`World::club_of` is the only club↔player reverse lookup**; `Club.players` is the one index.
- New fields on serialized types take `#[serde(default)]` so existing saves load.
