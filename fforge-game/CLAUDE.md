# fforge-game

Layer 5 (per DESIGN.md) of the fforge workspace: the CLI binary, consuming both
`fforge-domain` and `fforge-core`. `main.rs` is a thin presentation shell over
`fforge-core::Session` — it renders screens, reads menu input, turns choices into
`Command`s, and prints the resulting `Event`s. The Phase 1 walking skeleton is in
place: new/load game, squad/table/fixtures screens, lineup selection, matchday advance,
and JSON-lines save/load — matchday advance now runs the Phase 2a possession engine
under the hood via `fforge-core`, unchanged from this crate's point of view. Also
implements the Phase 2 "humble text match view" (`DESIGN.md` §9) for the human's own
fixture during matchday advance, paced line-by-line (via `crossterm`, this crate's one
non-workspace dependency) with a skip-to-full-time keypress when run in a real
terminal — piped/non-tty output (tests, redirects) still gets the whole stream at once.
The standalone friendly-match viewer (`watch_friendly_flow`) that also rendered this
view still exists but is currently unreachable from `game_loop`'s menu — the "watch a
friendly" option was removed (kept `#[allow(dead_code)]` rather than deleted).

The transfer-market menu (`TRANSFER_MODEL.md` §10's pre-commitment model) is wired in
as menu option `[9]`: browse candidate signings and the human's own squad (both priced
against one frozen `club_ai::observe`/`valuation::value_all` snapshot, rebuilt only on
entry and after a submit), build a local draft of `Bid`/`List` decisions, reorder it
(draft order is bid priority — `market::resolve_window` tries the first still-biddable
entry each round), and submit it in one shot via `Command::SubmitTransferDecision`.
Cash and wage headroom are shown in the screen header throughout, since those are what
`market::filter_affordable`'s resolve-time gate silently drops a plan on. When
`AdvanceMatchday` crosses a window's close date, `advance_flow` reports every
`Event::TransferCompleted` touching the human's club (in/out) plus a league-wide deal
count, right alongside that matchday's results — this is presentation only: nothing
here decides which transfers clear, `fforge-core::market` already did that inside the
event batch `AdvanceMatchday` returned. Deliberately function-only, no layout/IA work
(that is a later batch's job): plain numbered lists and `[key]`-style prompts, matching
the rest of `main.rs`'s existing screens.

## Function map (`main.rs`)

| Group | Functions | Does |
|---|---|---|
| Entry / flow | `main`, `new_game_flow`, `load_flow`, `game_loop` | top menu, world creation, save loading, the per-matchday menu loop |
| Screens | `header`, `squad_screen`, `table_screen`, `fixtures_screen`, `stats_screen`, `season_end_screen` | read-only renders of `Session` state |
| Lineup | `set_lineup_flow`, `auto_fill` | formation + XI picker, submits `Command::SubmitLineup` |
| Transfers | `transfer_flow`, `build_transfer_context`, `print_transfer_header`, `browse_targets_screen`, `prompt_role_filter`, `add_bid`, `squad_transfer_screen`, `toggle_list`, `shortlist_screen`, `decision_summary`, `submit_draft`, `prompt_money` | the §10 pre-commitment UI: builds/edits a local `Vec<TransferDecision>` draft against a frozen `ClubObservation`, submits it via `Command::SubmitTransferDecision` |
| Advance | `advance_flow`, `print_transfer_window_outcome` | calls `fforge_core::player_match_preview` on the pre-advance state to get the human's own match's trace, submits `Command::AdvanceMatchday`, renders that trace via `print_humble_text_view` before printing the matchday's plain results, then reports any transfer window this advance closed (`Event::TransferWindowClosed`/`TransferCompleted`) |
| Friendly (unreachable, `#[allow(dead_code)]`) | `watch_friendly_flow` | picks two clubs, runs `match_engine::play_match` directly (not through `Session::execute` — unrecorded, no `Event`), renders the raw event stream via `print_humble_text_view` — no longer wired into `game_loop`'s menu |
| Helpers | `print_humble_text_view`, `key_pressed_within`, `print_result`, `table_position`, `club_avg_ca`, `ordinal`, `do_save` | small pure/IO utilities used by the screens above; `key_pressed_within` polls for a keypress with a timeout (via `crossterm`) so `print_humble_text_view` can pace playback and skip on demand |
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
   `print_humble_text_view`'s terminal raw-mode toggling (for the skippable playback
   delay) is the same kind of edge-only concern — it's always paired
   (`enable_raw_mode`/`disable_raw_mode`) around the loop so canonical mode is restored
   before the next `read_line`-based prompt.
2. **All game-state mutation goes through `Session::execute`.** `main.rs` never mutates
   `GameState` fields directly — it builds a `Command`, calls `execute`, and renders
   whatever `Event`s or error comes back. This keeps the CLI a pure consumer of the
   event-sourced core.

## Testing

No `#[cfg(test)]` tests are expected in this crate — it's an interactive I/O shell, and
correctness of the simulation lives in `fforge-core`/`fforge-domain`'s test suites.
Verify changes by running `cargo run -p fforge-game` and walking the affected flow
manually.
