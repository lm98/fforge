# fforge-game

Layer 5 (per `DESIGN.md`) of the fforge workspace: the CLI binary, consuming both
`fforge-domain` and `fforge-core`. A thin presentation shell over `fforge-core::Session` —
it renders screens, reads menu input, turns choices into `Command`s, and prints the
resulting `Event`s.

## Current state

**Batch 4 is complete — U1–U7 and G1–G4 — plus season rollover.** Everything the
simulation resolves now has somewhere to be seen:

- the presentation layer is split by role, pure, and snapshot-tested;
- a semantic colour vocabulary is in place and adopted, one axis per screen;
- Phase 4's state has screens (finances with the `FinanceTick` trend, contracts,
  valuations, squad depth against the market's hard stabilizers);
- `fforge-core::news` is wired into the live loop as an inbox;
- Phase 2e is fully surfaced: tactics, fitness/availability, cards and injuries in the
  match stream, the substitution plan editor, ratings and form;
- seasons roll over, reporting the summer's development on the squad.

Not built, and deliberately: set pieces (deferred beyond 2e, `MATCH_MODEL.md` §11), and
anything Phase 5 owns (agents, scouting fog-of-war, the journalist renderer).

The `[r] Reports` screen is still just `SeasonTelemetry` — accurate, but thin, and now
cumulative across seasons rather than per-season. It is the obvious next thing to grow.

## Module layout — split by role, not by feature (R17)

```
main.rs        entry, game_loop, the grouped menu, the Observers bundle
screens/       read-only renders — each a pure fn returning String (R16)
flows/         multi-step interactions (new game, team sheet, transfers, advance, season, friendly)
render/        the Sem colour vocabulary, table layout, shared formatting
input.rs       the only functions that touch stdin
snapshots/     committed screen snapshots (see Testing)
```

**Screens are pure; flows are not.** A screen is a function of state that returns a
`String` and prints nothing — that is what makes the presentation layer snapshot-testable,
and it is the discipline `MatchEvent::commentary` already followed one layer down (build
the string in a pure function, let the caller do the I/O). A flow is a state machine over
player input: it prompts, loops, and eventually turns choices into a `Command`. `main`
prints screens with `print!`, since every screen's string already ends in a newline.

| Module | Holds |
|---|---|
| `main` | the entry menu, `game_loop`, R14's grouped menu, `SAVE_PATH`, and `Observers` |
| `screens::header` | club, matchday, date, position, and what is still outstanding (unset lineup, unread inbox, pending transfer plan) |
| `screens::squad` | the squad list with wage/contract/valuation/rating/form columns, plus depth against `SQUAD_TEMPLATE` and the market's hard stabilizers |
| `screens::availability` | condition, injuries with return dates, suspensions, card tallies — ordered unavailable-first |
| `screens::finances` | balance, reserve floor, wage ceiling, committed wages, headroom, and the monthly `FinanceTick` trend read straight off the log |
| `screens::inbox` | `news::NewsObserver`'s items via `TemplateRenderer`, ordered by salience with the background band capped separately |
| `screens::{table, fixtures, stats, season_end}` | league table, this/last matchday, season telemetry, the end-of-season wrap |
| `flows::new_game` | `new_game_flow` (seed → worldgen → club pick → `GameStarted`), `load_flow` |
| `flows::lineup` | formation + XI, then `flows::tactics`, then `flows::subs` — one `Command::SubmitLineup`, because they are one `Lineup` value |
| `flows::tactics` | the four-instruction picker, seeded from `ai_pick_tactics`'s read on the upcoming fixture |
| `flows::subs` | the bench and the condition→action rule list (`MATCH_MODEL.md` §16). The hardest screen in the game — see `docs/UI_TOOLKIT_EVIDENCE.md` §4 before changing it |
| `flows::season` | the season boundary: roll over via `Command::StartNextSeason` and report the summer's development, or stop |
| `flows::transfers` | the `TRANSFER_MODEL.md` §10 pre-commitment UI: a local `Vec<TransferDecision>` draft against one frozen `observe`/`value_all` snapshot, submitted in one shot |
| `flows::advance` | `player_match_preview` → `Command::AdvanceMatchday` → match view, results, and any transfer window this advance closed |
| `flows::match_view` | `DESIGN.md` §9's humble text match view plus the full-time aftermath (cards, injuries, man of the match); `commentary_lines`/`aftermath` are pure, `print_humble_text_view` adds pacing and skip-on-keypress |
| `flows::friendly` | the tactics sandbox: your club, an opponent you choose, any shape you like, nothing recorded |
| `flows::save` | `do_save` — saving is literally "serialize the event log" |
| `render::sem` | `Sem` and the **one and only** mapping from it to a colour |
| `render::table` | column layout — pads before it paints |
| `render` (root) | `money`, `ordinal`, `result_line`, `headline_ca`, `club_avg_ca`, `table_position` |
| `input` | `read_line`, `prompt_choice`, `prompt_menu`, `prompt_number`, `prompt_money`, `prompt_seed` |

## Hard constraints — never violate these

1. **This crate is the only place allowed to touch stdin/stdout and the wall clock.**
   `fforge-domain` and `fforge-core` must stay pure (see their own CLAUDE.md files). Two
   sanctioned wall-clock exceptions, both at the edge and both harmless for the same
   reason — nothing they produce ever reaches the fold: `input::prompt_seed`'s fallback to
   `SystemTime::now()` when the player leaves the seed blank (the chosen seed is
   immediately recorded in `Event::GameStarted`, so replay never re-derives it), and
   `flows::friendly`'s ad-hoc RNG seed (a friendly produces no `Event` at all).
   `print_humble_text_view`'s raw-mode toggling is the same kind of edge-only concern, and
   is always paired so canonical mode is restored before the next prompt.

2. **All game-state mutation goes through `Session::execute`.** Nothing here mutates
   `GameState` fields directly — it builds a `Command`, calls `execute`, and renders
   whatever `Event`s or error comes back.

3. **Screens never print, prompt, or mutate.** If a new screen needs to ask something, the
   asking half belongs in a flow. A screen that prints cannot be snapshot-tested, and the
   snapshots are the only thing standing between this crate and silent formatting rot.

4. **Nothing outside `render::sem` names a colour.** No `crossterm::style` import, no ANSI
   escape, anywhere else. Screens ask for `Sem::Warn`. This is how a codebase avoids green
   meaning "healthy" on one screen and "selected" on another.

5. **One semantic axis per screen, stated in a comment at the top of it.** That comment is
   what stops the vocabulary drifting back into decoration. If a screen genuinely needs a
   second meaning, use a *disjoint* colour set and say so explicitly — `screens::squad` is
   the worked example (ability uses `Good`/`Ok`/`Muted`; the depth block uses only `Bad`,
   so a red on that screen has exactly one meaning).

6. **Colour is never the sole carrier of information.** Every coloured distinction also has
   a glyph, a column, or an ordering saying the same thing. Verify by reading the
   *no-colour* snapshot: if a distinction is invisible there, colour is doing work alone
   and the screen needs a non-colour carrier.

7. **Pad before you paint.** An escape sequence has zero visual width and several bytes of
   it, so `format!("{:<20}", palette.paint(..))` pads to the wrong place. Use
   `render::table`, which enforces the ordering; if you must hand-roll, lay the line out
   whole and paint the finished string.

8. **Add every new observer to `Observers`.** An observer that misses events produces a
   quietly wrong inbox rather than a compile error, so the list of who must see the stream
   is written down in exactly one place.

9. **Don't re-derive a rule the core already owns.** The ban rule
   (`GameState::is_suspended`), the man of the match (`match_engine::man_of_the_match`),
   the squad template (`worldgen::SQUAD_TEMPLATE`), the substitution decision points
   (`match_engine::SUB_CHECKPOINTS`) and the form window (`GameState::recent_ratings`) are
   all read, never recomputed here. A second copy of a rule in the presentation layer is a
   copy free to disagree with the one that decides — and the player will believe the one on
   screen.

## Colour axes in force

| Screen | Axis |
|---|---|
| Squad | ability relative to this squad (plus `Bad`-only for a stabilizer breach). Contract urgency and form are deliberately **uncoloured** — see below |
| Fitness & availability | availability: fit / doubtful / injured / suspended |
| Match aftermath | consequence severity |
| Season rollover | direction of development |
| Finances | headroom — comfortable / tight / breached |
| Inbox | salience |
| Table, Fixtures, Season end | `Mine` and nothing else |
| Header | outstanding decisions |
| Transfers | affordability against cash and wage headroom |
| Tactics picker | departure from neutral (deliberately *not* good/bad — non-dominance is squad-conditional, `TACTICS_MODEL.md` §9) |
| League stats | none, deliberately — raw readings with no direction to act on |

**Twice now the one-axis rule has pushed a real signal off colour and onto a glyph or a
bare column** — contract urgency (U4) and recent form (G4), both on the squad screen. Both
were the right call, and both are recorded in `docs/UI_TOOLKIT_EVIDENCE.md` §4b as the
clearest evidence that this screen wants more encodings than a terminal has channels. If
you are about to add a third: don't colour it either, and add it to that list.

Colour is suppressed on `NO_COLOR` (any value), `--no-color`, or a non-tty stdout;
`--color` forces it back on for `| less -R`. The policy is a `Palette` value threaded from
`main`, never an ambient `static` — the same instinct the core applies to RNG and the
clock.

## Testing

Snapshot tests live in `screens/tests.rs`, with expected output committed as plain `.txt`
files under `snapshots/` (no test framework, no extra dependency — `assert_eq!` against
`read_to_string`). They run against one fixed seed, so the world, the fixtures, and every
result are reproducible.

```
cargo test -p fforge-game                       # verify
UPDATE_SNAPSHOTS=1 cargo test -p fforge-game    # regenerate, then READ THE DIFF
```

Three whole-suite invariants matter more than any individual snapshot:

- `no_ansi_escapes_when_colour_is_disabled` — with colour off, no screen emits an escape.
  That single test protects every piped consumer, CI logs included.
- `colour_changes_nothing_but_colour` — strip the escapes from a coloured render and you
  are back at the plain one, byte for byte. This is what makes the plain snapshots a
  *complete* record of what a screen says.
- `the_screens_with_an_axis_actually_colour` — a screen that silently stopped colouring
  would otherwise pass everything else.

Interactive flows are still verified by hand: `cargo run -p fforge-game` and walk the
affected flow.

## Related records

- `docs/UI_TOOLKIT_EVIDENCE.md` — R18's record of what these screens taught, the input
  `DESIGN.md` §10's toolkit question has been waiting on. Complete as of Batch 4, including
  the G3 measurement it was written to wait for. Add to it when a new screen teaches
  something.
