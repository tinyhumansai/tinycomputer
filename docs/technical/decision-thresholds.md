# Decision thresholds

Every constant a flow decision is thresholded on, with its value and where it
lives. [`decision-loops.md`](decision-loops.md) explains each loop;
[`specs/jev-wide-turns.md`](specs/jev-wide-turns.md) the wide strategy's.
Change a constant and its row together.

| Constant | Value | Where | Meaning |
|---|---|---|---|
| `DONE` | 0.75 | `act/mod.rs` | completion that ends a step after acting; also the bar for `verify`, `wait_for`, `if`, `repeat_until`, and for a stalled `do` step's screen already showing its result |
| `ALREADY_DONE` | 0.85 | `act/mod.rs` | completion that skips a step before acting |
| `BLOCKED` | 0.70 | `act/mod.rs` | obstacle probability that triggers dismissal |
| `LEANS_DONE` | 0.50 | `act/mod.rs` | completion under which a `finished` move is overruled after acting |
| `REGRESSION` | 0.25 | `act/mod.rs` | progress drop that triggers undo |
| `UNHELPFUL` | 0.20 | `act/mod.rs` | `helped` probability that triggers undo |
| `SHORTCUT_FLOOR` | 0.50 | `act/mod.rs` | least probability for pressing a shortcut |
| `ACT` | 0.70 | `view/mod.rs` | element choice used without re-asking |
| `NAMED_FLOOR` | 0.45 | `ground/mod.rs` | element choice used when its name is in the purpose |
| `CORROBORATED` | 0.80 | `ground/mod.rs` | corroboration that accepts a target alone |
| `AGREED` | 0.50 | `ground/mod.rs` | corroboration that accepts a target the re-ask agreed on |
| `SLOT_FLOOR` | 0.40 | `enter/mod.rs` | least probability for a slot assignment |
| `LOCATE_FLOOR` | 0.50 | `steps/mod.rs` | least probability for a `read`, `pick`, or `stop_before` target, an `extract`'s list, or a ranked card belonging to the list a `pick` picks from |
| `RANKED_CHECKS` | 8 | `steps/mod.rs` | cards an exact `pick` ranking puts first that are asked about, at once, for the first that belongs to the list picked from |
| `MAX_LISTS` | 6 | `steps/mod.rs` | lists an `extract` offers Jev when several show; past it, the longest six |
| `LIST_PREVIEW` | 3 | `steps/mod.rs` | first items of each list an `extract` shows Jev to tell the lists apart |
| `LIST_LEAN` | 0.30 | `steps/mod.rs` | least probability of the list Jev leaned to when it chose none clearly, for that list to be taken over the longest |
| `LIST_LEAD` | 3.0 | `steps/mod.rs` | times the next list's probability that list needs |
| `MAX_COLLECTED` | 12 | `wide/mod.rs` | saved variables every state recalls as `already_collected`, the most recent first kept |
| `COLLECTED_CHARS` | 120 | `wide/mod.rs` | characters of each saved value `already_collected` recalls |
| `FAILURE_CHARS` | 200 | `steps/launch.rs` | characters of a failure's own words (its first line) an `open` or `browse` step's note keeps after its code |
| `MIN_FLAT_ITEMS` | 3 | `tinycomputer-core` `surface/groups.rs` | same-role leaf siblings that make a list for `extract` and `pick` on a screen where nothing repeats by ordinal, as on a desktop tree |
| `CAP` | 20 | `ask/mod.rs` | most options in one Choice |
| `HEDGED` | 0.10 | `ask/answers.rs` | distance from an even chance within which a condition's calibrated yes/no says nothing either way, and defers to a crisp coverage |
| `CRISP_TOP` | 0.85 | `ask/answers.rs` | probability on a condition's top coverage level at which it stands alone for a hedged yes/no |
| `DO_TURNS` | 8 | `steps/mod.rs` | turns a `do` step may spend |
| `REFLECT_FLOOR` | 0.50 | `reflect.rs` | belief that a pressed `choose` left its choice, below which it is repaired, and failed if the repair does not take |
| `REPAIR_TURNS` | 4 | `reflect.rs` | turns one reflection repair may spend |
| `MAX_ACTIONS` / `MAX_CALLS` | 120 / 10000 | `mod.rs` | per-run caps on actions and Jev calls |
| `MAX_VOTES` | 9 | `vote.rs` | most framings one decision is asked in; a deliberated decision is widened up to it |
| `STALL_TURNS` / `MAX_IDLE_WAITS` | 3 / 2 | `act/mod.rs` | unchanged turns before a step ends: `AlreadyDone` when the screen already shows its result (at `DONE`, with the completion loop on), else failed; idle waits before Jev may not wait again |
| `MAX_OBSTACLES` / `MAX_UNDOS` | 2 / 2 | `act/mod.rs` | obstacles dismissed and undos run per step at most |
| `FIELD_ERROR` | 0.70 | `enter/mod.rs` | field-error probability that makes a slot be entered again |
| `NOT_ASKED` | 0.35 | `enter/mod.rs` | "the form asks for it" probability under which a slot with no field is taken as not asked for |
| `BLIND_PICK_MISSES` | 1 | `enter/mod.rs` | details no picker offered, on a screen with no editable field, after which the rest are not looked for one by one and the step fails |
| `OPENER_FLOOR` | 0.80 | `enter/mod.rs` | least belief Jev gives that a control named by a slot's word (a "Search" link for the slot "search") shows that slot's box, before `enter` presses it to reveal the box |
| `EMPTY_CHECKS` | 2 | `steps/mod.rs` | checks in a row, a wait apart, on which a page says it found nothing (`FOUND_NOTHING` in `steps/condition.rs`) before a `wait_for` fails |
| `SUGGESTION_FLOOR` | 0.5 | `steps/suggestion.rs` | least probability a suggestion Jev picks after typing needs before it is pressed; under it the text stays as typed |
| `MOST_SUGGESTIONS` | 12 | `steps/suggestion.rs` | most new rows one pick of an autocomplete's suggestion is asked over |
| `OPTION_EXTRA_WORDS` | 12 | `steps/matching.rs` | words beyond an option's own that a label may carry and still be the option; a label longer than that lists more than the option (a panel naming every row) and is not pressed for it |
| `MAX_MONTHS` | 12 | `steps/date.rs` | months a calendar is paged forward, at most, looking for a date's day |
| `HEADING_REACH` | 8 | `steps/date.rs` | nodes, in document order, a calendar's month heading ("October 2026") may sit from the arrow that pages it for paging to stop there; the same words further off are some other text |
| `BARE_DAYS` | 7 | `steps/date.rs` | days a calendar must show as bare numbers (a number and a fare, no month) for its heading to stop the paging; a calendar whose days name their month is paged by them alone |
| `DAYS_REACH` | 100 | `steps/date.rs` | nodes, in document order, those bare days may sit from the arrow that pages the calendar (two months' cells, day names, and headings); numbers further off (a results list's pages) are no calendar's days |
| `MAX_REPEAT_PRESSES` | 3 | `act/mod.rs` | presses of one control (by label and place, so a toggle's two looks count as one) or one key in one `do` step after which it is struck off for the step; a scroll is no press. Pressing a named control also strikes off its copies on the other items of its list (same label, another card of the same list), once the next look shows the press changed the screen, unless the step says all, every, each, or both, or chooses several items (a choosing verb in its first two words and a count of 2 to 20, or a number word, within three words before a plural: "choose 2 adjacent seats"); undoing that press lifts its own copies again |
| `MAX_IDLE_SCROLLS` | 1 | `act/mod.rs` | scrolls that showed nothing new after which a step's "scroll" move is taken as "activate": the screen already lists what lies below the fold |
| `LAYER_COVERS` | 3 | `front.rs` | controls something drawn over the window must cover, beyond what was covered before the press that opened it, on the same page, before it counts as a dialog the task opened (`surface` `layer`); a step that pressed inside such a dialog hands it back at the next step, a press the page refused (`NOT_ACTIONABLE`) counts as none, and opening an address forgets it |
| `CALENDAR_DAYS` | 7 | `front.rs` | day cells nothing covers (a grid cell, or a day number beside a month's name) that make what is in front a calendar: a press refused as covered behind a calendar the task opened and has pressed in since, in that step or one before, closes it and presses again, where any other dialog of the task's is worked within |
| `FRONT_CONTROLS` | 8 | `act/turns.rs` | most controls of the task's dialog in front a failed step's note names, so a rescue answers with one of them |
| `STEADY_HOLD` / `STEADY_CHECKS` | 0.65 / 3 | `steps/mod.rs` | belief a `wait_for` condition must keep, on checks in a row of one unchanged screen, to be taken as held under `DONE` |
| `HEDGE_AFTER` / `HEDGE_AFTER_LARGE` | 4 s / 5 s | `hedge.rs` | how long a framing runs before a copy of it is sent and the first answer of the two taken; the longer wait is for a request of `HEDGE_LARGE_BYTES` (32 KB) or more. Sage gets no copy |
| `QUORUM_VOTES` / `QUORUM_LEFT` | 7 / 2 | `quorum.rs` | a decision asked at least 7 ways is merged without its last 2 framings once those in settle every question; the 2 still run, and count as calls |
| `QUORUM_TOP` | 0.9 | `quorum.rs` | least probability every framing in gives the option a Choice or Score ranks first, the same option in each, for a quorum; the page kind needs only the same option |
| `SURE_YES` / `SURE_NO` | 0.97 / 0.08 | `quorum.rs` | a yes/no settles for a quorum when every framing in puts it at or above the first, or every one at or below the second: `UNDECIDED_BAND` beyond the highest (0.85) and lowest (0.20) thresholds one yes/no is read against; a belief pairing two answers can still fall within a band, and is widened past every framing asked |
| `HEDGE_COPIES` | 2 | `hedge.rs` | most copies one runtime has in flight: past it, a framing waits for its own answer, so a slow or failing gateway is not sent a copy of every call |
| `LATE_LOOKS` | 2 | `steps/suggestion.rs` | looks again, after a wait for the page to change, for the suggestions a place box lists late, until a row names the text typed (rows of the box's own, such as "Allow location access", are waited past), before its text is left as typed; a page that stayed still through a wait lists nothing more |
| `LATE_LOOK_MS` | 1000 ms | `steps/suggestion.rs` | longest one of those waits: it ends as soon as the page changes (`Surface::await_change`) |
| `BARE_CHARS` | 3 | `tinycomputer-core` `surface/groups.rs` | most letters and digits each field of a card may show for a list of such cards to be bare markers (carousel dots, size chips, page numbers), never results |
| `CARD_LINK_CHARS` | 20 | `tinycomputer-core` `surface/groups.rs` | least characters (with a word in them) a link, option, radio, or button must show for a run of three or more under one parent to be a list of cards that are one control each |

## Deliberation

The gates of [`specs/jev-deliberation.md`](specs/jev-deliberation.md), which
replace the single-number bars above at every site they apply to unless
`deliberation` is `off`.

| Constant | Value | Where | Meaning |
|---|---|---|---|
| `MAX_DISTRACTIONS` / `MAX_CLEARED` | 4 / 3 | `attention/` | distractions one attention question offers; distractions cleared per step |
| `MAX_DISTRACTION_SIZE` | 12 | `attention/` | most elements a distraction holds; a bigger container, or one with more than one text field, is the page or a form |
| `ATTENTION_FLOOR` | 0.50 | `attention/` | least probability a distraction must win the attention Choice with, beside the gate's margin and agreement |
| `ACCEPT_MARGIN` | 0.25 | `evidence/` | least lead of a Choice's winner over the runner-up to act on it as read |
| `ACCEPT_AGREEMENT` | 0.80 | `evidence/` | least share of framings that picked the winner, or put a judgement on the same side of its threshold, to act on it as read |
| `ABSTAIN_FLOOR` / `ABSTAIN_AGREEMENT` | 0.20 / 0.40 | `evidence/` | a winner under both is abstained from: nothing serves |
| `UNDECIDED_BAND` | 0.12 | `evidence/` | half-width of the band around a judgement's threshold inside which it is deliberated |
| `MAX_FINALISTS` / `FINALIST_FLOOR` | 4 / 0.05 | `duel/` | most finalists a duel compares, and the least probability to be one |
| `DUEL_WIN` | 0.60 | `duel/` | least share of every pairing the champion must take, both orders averaged |
| `CONTRAST_ACCEPT` / `CONTRAST_LEAD` | 0.65 / 0.20 | `escalate/` | with no duel champion, the belief and lead a contrasted leader needs to be taken over the duel's ranking |
| `BRANCH_MARGIN` | 0.30 | `ground/mod.rs` | lead of the chosen region under which narrowing keeps the runner-up region too |
| `MISTAKE` | 0.50 | `act/mod.rs` | "did what it was meant to" belief under which a press whose effect was missed is undone |
| `CLEAR_MISTAKE` | 0.25 | `act/mod.rs` | that belief under which any press is undone |
| `MAX_BRANCHES` | 3 deep / 1 standard | `act/mod.rs` | next-best candidates a `do` step backtracks into |
| `RESTORED` | 0.80 | `checkpoint/` | share of a checkpoint's marks a screen must show again for an undo to count as verified |
| `IRREVERSIBLE_FLOOR` | 0.85 | `steps/mod.rs` | belief a deep run needs before a `stop_before` presses its control |
| `CROWDED` | 40 | `survey.rs` | actionable elements above which a wide turn surveys the screen first |
| `DISTRACTION` | 0.70 | `survey.rs` | distraction probability that collapses a region and ranks it last |
| `DIGEST_BUDGET` | 24,000 bytes | `wide/mod.rs` | screen a wide request shows before regions are collapsed (about 9,000 tokens of dense page text) |
| `WIDE_POOL` | 40 | `wide/mod.rs` | candidates one move is offered in a wide turn: two Choices of `CAP` |
| `NEW_TENTHS` | 3 | `survey.rs` | tenths of a page's regions that must be new before a step surveys it again |
| `REGION_SIZE` | 24 | `tinycomputer-core` `surface/digest/` | elements a region holds before it is split one level deeper |
| `MAX_DEPTH` | 10 | `tinycomputer-core` `surface/digest/` | deepest ancestor level regions are split on |
| `LIST_CARDS` | 12 | `tinycomputer-core` `surface/digest/` | cards of a list shown one line each before the rest are counted |
| `CARD_CHARS` | 160 | `tinycomputer-core` `surface/digest/` | longest a card's line is let to run |
| `EXAMPLES` | 5 | `tinycomputer-core` `surface/digest/` | example labels a collapsed region names |
| `SUMMARY_CHARS` | 200 | `tinycomputer-core` `surface/digest/` | longest a collapsed region's one-line summary is let to run |
| `COLLAPSED_SLACK` | 400 bytes | `tinycomputer-core` `surface/digest/` | how far collapsed summaries push `spent` past the render budget before the rest are reported as one count instead |
| `MAX_FINISHED` | 40 | `ledger.rs` | finished steps the ledger keeps before the oldest is dropped |
| `MAX_RECENT` | 24 | `ledger.rs` | history lines shown as `recent_actions` |
| `MAX_TRIED` | 12 | `ledger.rs` | `tried_and_failed` notes kept before the oldest is dropped |
| `MAX_LINE` | 240 | `ledger.rs` | longest a ledger line is let to run |
