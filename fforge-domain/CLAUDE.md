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
| `character` | `Character` (hidden attributes: potential, determination, professionalism, consistency, injury_proneness, natural_fitness, leadership — `natural_fitness` added at Phase 2e, `MATCH_MODEL.md` §13/R8, `#[serde(default)]`) |
| `entities` | ID newtypes, `Player` (incl. `DevProfile`, `contract: Option<Contract>`, `retired`), `Staff`, `Club` (incl. `coaching_milli`, `finances: Finances`, `reputation: u8`), `Competition`, `Fixture`, `World` (incl. `World::club_of(PlayerId) -> Option<ClubId>` — the sole club↔player reverse lookup; `Club.players` stays the one index). `DevProfile` = the once-resolved Phase-3 development trajectory (fixed-point `E`/`φ`, DEVELOPMENT_MODEL.md §2.3); `Club::coaching_milli` = per-club academy quality (§3) — both float-free, resolved at worldgen. `Money` (signed `i64`, whole currency units), `Contract` (`wage: Money`, `expires: GameDate`), and `Finances` (`balance: Money`, `wage_budget: Money`) are the Phase-4 finance types (`TRANSFER_MODEL.md` §3) — the sanctioned exception to "no new features at this stage," kept to the minimum that earns its keep |
| `date` | `GameDate` — flat 365-day sim calendar, no leap years, no wall clock |
| `formation` | `FormationDef`, `Lineup` (carries `tactics: Tactics` — `TACTICS_MODEL.md` §6 — and, since `MATCH_MODEL.md` §16/T12, `bench: Vec<PlayerId>` and `sub_plan: Vec<SubRule>`, all three `#[serde(default)]`), `FORMATIONS` — 4 hardcoded formations, `BENCH_SIZE` (7), `MAX_SUBSTITUTIONS` (3 — the batch-3 T12 task spec's own cap, not the real law's 5 `MATCH_MODEL.md` §16 first drafted; see that section's T12 finding) |
| `tactics` | `Tactics` + its four ternary instruction enums (`Mentality`/`Tempo`/`Width`/`Pressing`, `TACTICS_MODEL.md` §2) — `Default`/`neutral()` is all-`Balanced`, the sanctioned Phase-2e domain extension (`MATCH_MODEL.md` §12) |
| `substitution` | `ScoreState`, `SubCondition` (`MinuteAtLeast`/`Score`/`PlayerConditionBelow`/`PlayerInjured`/`ManDown`), `SubAction` (`Substitute`/`SetMentality`/`SetTempo`/`SetWidth`/`SetPressing`), `SubRule { conditions: Vec<SubCondition>, action }` — the pre-committed condition→action substitution/tactic-change plan `Lineup.sub_plan` carries (`MATCH_MODEL.md` §16, `TACTICS_MODEL.md` §7's pre-commitment pattern applied to the match clock), the sanctioned Phase-2e domain extension alongside `tactics` |

`lib.rs` re-exports everything public; consumers import from the crate root.
