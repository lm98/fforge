# UI toolkit evidence — what Batch 4's screens taught

`DESIGN.md` §10 holds the egui-vs-Tauri question open with an explicit condition: *held
until the management screens have been felt in practice.* Batch 4 was that practice. This
document is the record it was supposed to produce.

**This is not a decision.** Batch 4 gathers; Phase 6 decides. Nothing here recommends a
toolkit. What it does is write down, while the memory is fresh, which screens the terminal
handled well, which it fought, and what the fighting actually cost — because the question
was deferred *to collect exactly this evidence*, and evidence collected and not written
down is evidence that evaporates.

Scope note: all of Batch 4 is built — U1–U7 and G1–G4 — plus season rollover. **G3, the
substitution rule builder, was the measurement this document was written to wait for**, and
§4 now records what it actually cost rather than what it was predicted to cost.

---

## 1. What the terminal handled well

**Tabular, read-only, one row per entity.** Squad, league table, fixtures, the transfer
browser, the depth summary. These are the CLI's home ground and it shows: `render::table`
is ~200 lines including tests, and every one of these screens is a `for` loop over rows.
There is no layout problem to solve, because a monospace grid *is* the layout.

Two specifics worth carrying forward, because they are properties of the *data*, not of the
terminal, and will still be true in a GUI:

- **Column width is a fixed, known quantity.** Club names cap around 20 characters, money
  abbreviates to five (`12M`, `250k`), CA is two digits. Nothing in this domain wants an
  elastic column. A GUI would gain nothing here it does not already have.
- **The sort order is usually the whole interaction.** The squad screen sorts by role then
  ability; the transfer browser by value; the inbox by salience. In every case, once the
  right sort is chosen the player's question is answered by the *top few rows*. That is a
  strong hint that sortable-column headers matter more than any other GUI affordance we
  might reach for.

**Single-axis colour worked, and worked better than expected.** R15's discipline — one
semantic axis per screen, colour always redundant with a glyph, column, or ordering —
produced screens that stay readable with `NO_COLOR` set. The constraint that made this work
is `render::sem`: exactly one module maps `Sem` to a colour, so the vocabulary cannot drift
into decoration one screen at a time. **That module is toolkit-independent and should
survive a rewrite verbatim.** Whatever renders the pixels, `Sem::Warn` is the same idea.

**Snapshot-testable output turned out to be the single most valuable structural decision in
the batch** (R16/U1). Screens are pure functions returning `String`; `main` prints. Every
formatting regression in U3 and U4 was caught by a diff rather than by a person noticing.
Two of those catches were real bugs neither reading nor manual testing would have found:
trailing padding being trimmed *after* colour was applied (so the spaces sat inside the
escape pair and `trim_end` could not see them), and a column-alignment test using byte
offsets on a line containing "Atlético".

**The implication for Phase 6 is not about terminals at all:** whatever the toolkit, the
render should stay a pure function of state producing an inspectable value. In egui that
means resisting the immediate-mode temptation to compute state inside the frame closure; in
a web front end it means a serialisable view model. The property worth preserving is *the
render is testable without a display*.

---

## 2. What the terminal fought

### 2.1 Simultaneous panes — the recurring loss

The clearest and most repeated finding. Every time a screen needed a second region visible
at the same time, the CLI's answer was "scroll up", which is not an answer.

- **Lineup selection** picks eleven slots in sequence. The squad you are picking *from* is
  not on screen while you pick — only the top eight candidates for the current slot. You
  cannot see the XI taking shape beside the pool you are drawing from, which is exactly
  the comparison the task consists of.
- **Tactics** (U6) is four instructions whose interactions are the interesting part
  (`TACTICS_MODEL.md` §3 is explicit that they compose). The picker re-prints all four every
  cycle — the right call in a terminal — but a player tuning `Pressing` cannot watch what it
  does to anything else, because nothing else is shown changing.
- **Transfers** is the worst case. The decision is "can I afford him, and who would I sell
  to make room", and that is three regions: the target list, your own squad, and the two
  headroom figures. The CLI has them on three separate screens. The mitigation built in U3
  — a `Fit` column naming which half of the affordability gate blocks each row — is a
  *good* mitigation and it exists only because the real answer (show the headroom next to
  the list, live) was unavailable.

- **The match aftermath** (G2). A match is a sequence, which the terminal handles well —
  but the cards, injuries and man of the match want to sit *beside* the stream, not after
  it. The only available answer was a summary block at full time.

**This is the strongest single argument in the document for a windowed toolkit**, and it is
not a taste argument: four of the batch's screens each independently reinvented a workaround
for the same missing capability.

### 2.2 Live-updating regions

Related but distinct. The transfer flow freezes its valuation snapshot on entry and
rebuilds it only after a submit — which is *correct* modelling (`TRANSFER_MODEL.md` §2.7: a
real window re-prices once, at close), but the CLI cannot show a draft's running effect on
headroom as you build it. You add three bids and the headroom line does not move until you
submit. A GUI would let the draft's projected cost sit live beside the budget, which turns
a submit-and-see loop into a direct-manipulation one.

The match view has the same shape from the other end: `print_humble_text_view` paces the
event stream line by line, and pacing is the *only* dynamic thing the terminal does well.
It works because a match is inherently a sequence. Nothing else in the game is.

### 2.3 Form versus browse — the ratio is not what a terminal is built for

Counting the batch's screens: **six are browses** (squad, table, fixtures, finances, inbox,
transfer targets; seven with fitness & availability) and **four are forms** (lineup,
tactics, transfer draft, substitution plan). The terminal is excellent at the browses and
mediocre at
the forms, and the forms are where the *decisions* live. A management game is a decision
game; the browses exist to support the forms.

The specific weakness is that a CLI form is a **state machine over keystrokes** with no
persistent representation of the form itself. The transfer draft (`shortlist_screen`) is
the honest illustration: reordering entries is `u 3` — "move entry 3 up one" — which is a
command language invented because drag-and-drop was unavailable, and which the player must
be *taught*. Every other form in the batch has a similar small invented vocabulary
(`d N`, cycle-on-keypress, `f` to filter). Each is fine alone. Together they are a dialect.

### 2.4 Where linearity actively fought the task

Two concrete cases, both about **losing work to a wrong keystroke**:

1. **The lineup flow is all-or-nothing, and G3 made it worse.** Aborting the tactics picker
   — or now the substitution editor, a step further in — throws away the whole team-sheet
   submission, because the XI, the tactics, the bench and the plan are one `Lineup` value
   (`TACTICS_MODEL.md` §6, `MATCH_MODEL.md` §16) and there is nowhere to park a half-made
   one. That is the right *model*; it is the wrong *interaction*, and the cost grows with
   every step added to the sheet. A GUI would keep it resident and let you change any part.
2. **There is no "back" anywhere.** Slot 7 of 11 chosen wrongly means `q` and start again.
   Adding an undo to a linear prompt chain means inventing a history stack the terminal
   gives you nothing toward.

---

## 3. Things that will be true regardless of toolkit

Recorded here because they are easy to lose in a rewrite:

- **`Sem` and its one-module mapping.** Toolkit-independent; port it as-is.
- **Screens as pure functions of state.** The testing property, not the terminal property.
- **Colour is never the sole carrier.** Restated for a GUI: an icon-only or colour-only
  status is the same bug.
- **The valuation-visibility asymmetry is a design question, not a UI one.** The squad
  screen labels `Value*` as omniscient ground truth (`TRANSFER_MODEL.md` §2.6) because
  scouting fog-of-war is Phase 5. Whatever the toolkit, Phase 5 has to decide whether the
  human is fogged like the agents or privileged over them. That is a game-feel question and
  it will arrive with or without a GUI.
- **Money wants abbreviating.** `1.5M`, not `1500000`. Magnitude is the point on these
  screens; precision never is.

---

## 4. The decisive measurement: G3, the substitution rule builder

The hypothesis, recorded before the screen was built: *G3 will need an invented command
dialect at least as large as the transfer draft's, and will be the first screen in the game
a player cannot use without being told how.*

**Both halves came in true, and the second more strongly than expected.**

**The dialect.** The transfer draft builder invented four commands (`d N`, `u N`, `c`, `q`).
The plan editor invented eight at its top level (`b`, `a`, `n`, `d N`, `u N`, `c`, `k`, `q`)
*plus* a nested chain of numbered pickers: action type → (for a substitution) who comes off →
who comes on → condition type → condition parameter → done. Authoring one rule is four
prompt levels deep, and nothing above the current level is on screen while you are in it.
`flows/subs.rs` is 536 lines against `flows/transfers.rs`'s 473 — and transfers is *three*
screens plus a submit path, where subs is one.

**Told how.** Every other screen in the game is usable from its own contents. This one is
not, and the code says so: the editor spends a header line explaining *when* rules are
evaluated (half-time, 60', 70', 80', and immediately after an injury or card), because
without it a `MinuteAtLeast(65)` rule looks like it fires at 65 when it fires at 70. It
needed `match_engine::SUB_CHECKPOINTS` made public purely so the screen could avoid lying
about the mechanism it drives. Three more facts have no natural home either — rules fire in
list order, only three substitutions can ever be used, and a rule naming a player no longer
on the team sheet silently does nothing.

**What it cost to make usable anyway**, which is the part worth carrying into Phase 6,
because each of these is a workaround for a specific missing capability:

| Compensation | Missing capability |
|---|---|
| The plan is re-rendered *in full, as English prose*, before every prompt | a persistent form widget you can look at while editing it |
| Rules are seeded from last week's plan | any notion of a document that stays open |
| Stale rules are detected and labelled `(stale)` | live validation against the form's own other fields |
| An advisory when substitution rules outnumber `MAX_SUBSTITUTIONS` | a constraint the widget could simply enforce |

Prose-rendering the whole plan on every keystroke is the load-bearing one, and it is a
genuinely good idea that a GUI would make unnecessary: it exists *because* there is nowhere
to put a form.

**One honest caveat against §2's weight.** The prose rendering works well. Reading

> `2. if it is 70' or later and Rossi is under 60% fitness — bring Bianchi on for Rossi`

is arguably clearer than the same rule as a row of dropdowns would be. So the terminal's
loss here is not about *reading* the plan — it is entirely about *building* and *revising*
it. Phase 6 should weight §2.3 accordingly: the deficit is in authoring, not display.

## 4a. What the other gated screens added

- **G1 (fitness & availability)** is a plain table and the terminal handled it perfectly —
  more confirmation of §1 than new information.
- **G2 (cards, injuries, subs in the match view)** reinforced §2.1 from a new angle. A match
  is a sequence, which is the one dynamic thing a terminal does well; but the *aftermath*
  wants to sit beside the stream rather than after it, and the only available answer was to
  append a summary block at full time. Another second-pane workaround.
- **G4 (ratings and form)** produced the batch's clearest recurring finding, below.

## 4b. The squad screen wants more encodings than one channel can carry

Twice now, R15's one-axis rule has forced a real signal off colour and onto a glyph or a
bare column: **contract urgency** at U4, and **recent form** at G4. Both were correct calls —
two hues meaning two different things on one list is the ambiguity the rule exists to
prevent — but twice is a pattern, not a coincidence. The squad screen carries eight columns
and at least three independent things a manager reads it for (who is good, whose deal is
running out, who is in form), and a terminal offers exactly one colour channel to spend.

**This is the most specific toolkit-relevant finding in the document**, because the GUI
answer is concrete and cheap: sortable column headers plus a per-column colour scale. Each
column gets its own encoding, so the one-axis rule stops binding at all — it is a constraint
of the medium, not of the domain. §1 already noted that sortable headers looked like the
highest-value affordance; G4 is the second independent argument for the same conclusion.

---

## 5. Summary of the evidence, without a recommendation

| Finding | Strength | Where |
|---|---|---|
| Tabular browses are well served by a terminal | Strong | §1, §4a |
| Four separate screens reinvented a workaround for "no second pane" | **Strong** | §2.1, §4a |
| Live-updating regions are unavailable; the transfer draft feels it most | Moderate | §2.2 |
| Forms outnumber browses where the decisions are, and forms are the weak side | Moderate | §2.3 |
| Linear prompt chains lose work and have no undo | Moderate | §2.4 |
| G3 confirmed its own pre-registered hypothesis: largest dialect, needs explaining | **Strong** | §4 |
| ...but the terminal's deficit there is in *authoring*, not *display* | Moderate (counter) | §4 |
| One colour channel is not enough for the squad screen; sortable columns are the fix | **Strong** | §4b |
| The `Sem` vocabulary and pure-render discipline should survive any rewrite | Strong | §3 |

---

## 6. What the highlight reel taught (a Phase 6 slice, pulled forward)

**Scope note, so the phase ordering is not read as a mistake.** `DESIGN.md` §9 puts UI/UX
at Phase 6, after the agent layer. The batch recorded here is a deliberate early slice of
it — the match view, the status header, the club picker, the title screen — taken because
the project is a *game* and the thing being built had stopped looking like one. Nothing in
the simulation changed. What follows is what the slice taught, in the same register as the
rest of this document: evidence, not a decision.

### 6.1 The strongest finding: the stream is not the telling

The humble text match view printed **867 events** for a 90-minute match, the overwhelming
majority of them "picks a pass in midfield". It is a faithful rendering of the stream and
an unwatchable rendering of a football match, and the two facts are not in tension —
`DESIGN.md` §9 asked for a proof that the stream *can* carry the story, and it does.

What the slice added is a second consumer at a different altitude: a **highlight reel**
(`flows::match_view::highlights`) that keeps shots, goals, cards, injuries and
substitutions, drops the three kinds of duplicate beat (a `Save` the shot line already
narrates, a `Cross` the header narrates one line later, a long shot that misses), and
carries a running scoreline in a gutter. Same match, same `MatchOutcome`, ~27 lines instead
of 867.

**The finding worth carrying into Phase 6 is the ratio.** 867 → 27 is a filter doing 97% of
the work of making the same data legible, and it needed no new state, no engine change, and
no schema change — only a predicate over `MatchEventKind`. That is the payoff §9 predicted
when it insisted the stream be designed for narratability rather than for outcomes, and it
is the first time the prediction has been *cashed*. Any future viewer — graphical,
journalist-agent, or otherwise — is another predicate over the same alphabet, not another
engine.

The corollary is a warning: **the alphabet is now load-bearing in a way it was not before.**
`is_highlight` is a rule that lives in the presentation layer and is stated once, and every
new `MatchEventKind` has to be classified by it or it silently vanishes from what a player
sees. That is a maintenance cost the raw view never had.

### 6.2 Match statistics are free, and they were the missing altitude

Possession, shots, shots on target, fouls and cards are all countable off the stream with
no new state at all (`flows::match_view::stats`). They fill the gap between the reel (what
happened) and the scoreline (what it came to) — and the one non-obvious detail is worth
recording, because getting it wrong would have been invisible: a `Foul` beat's `side` is
the **fouled** side (`MATCH_MODEL.md` §15), so the discipline columns are counted against
the *other* side's beats. Two columns that read plausibly either way is exactly the sort of
bug a snapshot cannot catch, so it has its own test.

### 6.3 A terminal *can* do drama, if the pacing is a function of match time

§2.2 recorded that pacing is "the only dynamic thing the terminal does well". The reel
sharpens that: pacing at a fixed delay per line reads as a machine printing; pacing
*proportionally to the match minutes between beats* reads as a match. A goalmouth scramble
arrives in a burst and a quiet twenty minutes is a pause, from the same data, with a clamp
at each end so neither degenerates. This is the one place in the game where the terminal
does something a naive GUI would not do better for free, and it is worth keeping whatever
Phase 6 decides.

### 6.4 A framed panel resolved §4b's pressure without spending a colour

The status header now carries five readings — competition, matchday, position, points,
next opponent, recent form — in a fixed-width box. **Form is deliberately uncoloured**,
which is the third instance of the pattern §4b named: a real signal pushed off colour and
onto its own glyph column because the screen's one channel is already spoken for.

But the panel is also the first counter-evidence in this document to §4b's conclusion.
`W W W W L` in a fixed column, aligned under the same frame every turn, is *legible without
colour at all* — the letters are the encoding. Where §4b's squad screen wanted a per-column
colour scale because its columns are continuous quantities (ability, wage, value), the
header's are categorical and short, and categorical-and-short does not need a hue. **The
sharpened claim: it is continuous columns that exhaust a terminal's one channel, not
columns in general.** Phase 6 should read §4b with that qualifier attached.

### 6.5 Two things that were not UI problems at all

Both found while curating, both worth naming because a GUI would not have fixed either:

- **Dates.** `GameDate`'s `Display` is `2026, day 220`. Nothing about that is wrong for a
  log line and everything about it is wrong for a football screen. The fix was a
  presentation function in layer 5 (`render::date` → `9 Aug 2026`) rather than a change to
  the domain type, because the sim genuinely has no months and should not acquire any. The
  flat 365-day year happens to be tiled *exactly* by the twelve non-leap month lengths, so
  the mapping is total and lossless — a piece of luck, but a checkable one.
- **End of input hung the game.** Every prompt looped until it liked its input, and EOF
  reads as an empty line forever, so a redirected run played out the rest of the season and
  then span. `input::read_line` now returns `Option`, and the convention is stated once: a
  prompt's *last* allowed option is its way out, and EOF takes it. That is a robustness bug
  a windowed toolkit would never have surfaced, because a window has no EOF.
