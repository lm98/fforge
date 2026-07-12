# fforge-game

Layer 5 (per DESIGN.md) of the fforge workspace: the CLI binary, consuming both
`fforge-domain` and `fforge-core`. `main.rs` is a thin presentation shell over
`fforge-core::Session` — it renders screens, reads menu input, turns choices into
`Command`s, and prints the resulting `Event`s. The Phase 1 walking skeleton is in
place: new/load game, squad/table/fixtures screens, lineup selection, matchday advance,
and JSON-lines save/load — matchday advance now runs the Phase 2a possession engine
under the hood via `fforge-core`, unchanged from this crate's point of view. Also
implements the Phase 2 "humble text match view" (`DESIGN.md` §9), both as a standalone
friendly-match viewer and, during matchday advance, for the human's own fixture.

## Function map (`main.rs`)

| Group | Functions | Does |
|---|---|---|
| Entry / flow | `main`, `new_game_flow`, `load_flow`, `game_loop` | top menu, world creation, save loading, the per-matchday menu loop |
| Screens | `header`, `squad_screen`, `table_screen`, `fixtures_screen`, `stats_screen`, `season_end_screen` | read-only renders of `Session` state |
| Lineup | `set_lineup_flow`, `auto_fill` | formation + XI picker, submits `Command::SubmitLineup` |
| Advance | `advance_flow` | calls `fforge_core::player_match_preview` on the pre-advance state to get the human's own match's trace, submits `Command::AdvanceMatchday`, renders that trace via `print_humble_text_view` before printing the matchday's plain results |
| Friendly | `watch_friendly_flow` | picks two clubs, runs `match_engine::play_match` directly (not through `Session::execute` — unrecorded, no `Event`), renders the raw event stream via `print_humble_text_view` |
| Helpers | `print_humble_text_view`, `print_result`, `table_position`, `club_avg_ca`, `ordinal`, `do_save` | small pure/IO utilities used by the screens above |
| Input primitives | `read_line`, `prompt_choice`, `prompt_number`, `prompt_seed` | the only functions that touch stdin |

## Hard constraints — never violate these

1. **This crate is the only place allowed to touch stdin/stdout and the wall clock.**
   `fforge-domain` and `fforge-core` must stay pure (see their own CLAUDE.md files). Two
   sanctioned wall-clock exceptions: `prompt_seed`'s fallback to `SystemTime::now()`
   when the player leaves the seed blank (the chosen seed is immediately recorded in
   `Event::GameStarted`, so replay/`fforge-core` never re-touches the clock), and
   `watch_friendly_flow`'s ad-hoc RNG seed (a friendly is never recorded — no `Event`,
   nothing for replay to reproduce). Any new randomness or timestamp need must be
   sourced here and passed in as data, never added to `fforge-core`/`fforge-domain`.
2. **All game-state mutation goes through `Session::execute`.** `main.rs` never mutates
   `GameState` fields directly — it builds a `Command`, calls `execute`, and renders
   whatever `Event`s or error comes back. This keeps the CLI a pure consumer of the
   event-sourced core.

## Testing

No `#[cfg(test)]` tests are expected in this crate — it's an interactive I/O shell, and
correctness of the simulation lives in `fforge-core`/`fforge-domain`'s test suites.
Verify changes by running `cargo run -p fforge-game` and walking the affected flow
manually.
