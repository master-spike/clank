# clank

A terminal typing trainer that measures what actually slows you down.

Most typing tutors (keybr and friends) track per-key averages of raw
inter-keystroke intervals. That conflates two different things: how slow a
*transition* really is, and a per-key *timing habit* — some keys you simply
tend to press "late", stealing time from one interval and refunding it to the
next without slowing you down at all. clank separates the two, drills you on
the transitions that are genuinely slow, and reports speed and accuracy
normalized against real English letter-pair frequencies so drilling hard
material never tanks your score.

## Install & run

Requires a Rust toolchain (edition 2024).

```sh
cargo run --release
```

Keys:

| Key         | Action                                   |
| ----------- | ---------------------------------------- |
| `a`–`z`     | type the shown lesson                    |
| `Backspace` | clear an unresolved mistake / step back  |
| `Tab`       | skip to a fresh lesson                   |
| `Esc`       | quit (model is saved)                    |

Your model is persisted to your platform data directory (on Linux,
`~/.local/share/clank/model.json`) at every lesson boundary and on exit, so
progress accumulates across sessions.

Run the tests with `cargo test`.

## The statistical model

### Bias-invariant digram speeds

For the `k`-th observation of key `i` followed by key `j`, the raw
inter-keystroke interval is decomposed as

```
T_ijk = s_ij + b_i − b_j + ε_k
```

- `s_ij` — the intrinsic time cost of the transition `i → j` (what we care about)
- `b_i`  — the delay bias of key `i` (a habit of pressing it late or early)
- `ε_k`  — zero-mean measurement noise

If a habit (or hardware quirk) shifts every press of key `i` late by `t`,
intervals *ending* at `i` grow by `t` and intervals *starting* at `i` shrink
by `t`. In this decomposition that shift is absorbed entirely by `b_i ← b_i − t`,
leaving every `s_ij` unchanged — so a "late" key is not mistaken for a slow
one. Summing intervals over a sequence telescopes the biases away, which
matches the intuition that a late-biased key doesn't cost you any total time.

Two details make this identifiable in practice:

- **Anchoring.** As specified above the model is over-parameterized: every
  dataset can be fit with all biases at zero. clank therefore fits the biases
  against the *global mean* interval (an ANOVA-style antisymmetric
  decomposition), and pair speeds against the bias-corrected residual. Biases
  are also re-centered to mean zero periodically, since they are only
  identifiable up to a shared constant.
- **Online updates.** Everything is learned by O(1) online gradient steps per
  keystroke — no matrix solves — with a count-based learning rate (fast on new
  pairs, settling to a floored constant so estimates never go fully sticky),
  residual clamping for robustness, and outlier rejection (pauses > 2 s, and
  any interval adjacent to an error or correction, are discarded).

### Normalized scores

Reported WPM is **not** your raw session speed. It is the model's expected
speed over the *population* distribution of English digrams (built from the
word list, Zipf-weighted by rank):

```
wpm = 60000 / (5 × Σ f_ab · s_ab)     f = population digram frequency
```

Because the weights are fixed population frequencies rather than whatever the
scheduler happens to be showing you, your score doesn't drop when clank feeds
you difficult material — it only moves when your underlying pair speeds move.
Accuracy is normalized the same way from per-pair error rates (with a small
prior for rarely-seen pairs).

### Error classification

The cursor does not advance past an unresolved mistake, which makes real-time
classification well-defined via one-key lookahead. With expected char `e`,
next `n`, and the one after `n2`, a mismatched key `w` resolves as:

| You typed             | Classified as                            |
| --------------------- | ---------------------------------------- |
| `w`, then `e` (w ≠ n) | **insertion** — an extra letter was hit  |
| `w`, then `n`         | **substitution** — `w` in place of `e`   |
| `w = n`, then `e`     | **reversal** — adjacent letters swapped  |
| `w = n`, then `n2`    | **omission** — `e` was skipped           |

Omissions, substitutions, and reversals re-align automatically so you can keep
typing; insertions resolve on the next correct key without backspacing. Every
error counts against the accuracy of the transition it occurred on, and breaks
the timing chain so no fabricated interval pollutes the speed model.

### Adaptive lessons

Digrams are ranked by expected payoff of practice:

```
payoff = √(population frequency) × speed deficit vs your mean × uncertainty boost
```

Lessons are sampled as a mix: 50% words containing a current focus pair (via
an inverted digram → word index, skewed toward common words), 30% weighted by
overall word difficulty, and 20% uniform exploration so no pair drops out of
rotation. The current focus pairs and their speeds are shown live in the UI.

## UI

- header — normalized + raw WPM and accuracy
- lesson line — green/red feedback, red cursor on an unresolved mistake
- errors — live counts by kind (extra / skipped / typo / swap) and the last event
- focus — the digrams currently being targeted, with their speeds
- per-key table — WPM and accuracy for each letter, heatmap-colored
- biases — the largest per-key late/early tendencies the model has isolated

## Word list

`assets/words.txt` is the [google-10000-english](https://github.com/first20hours/google-10000-english)
list, derived from the Google Web Trillion Word Corpus (Linguistic Data
Consortium). Educational and personal/research use is permitted; commercial
use may require an LDC license. It is not covered by this repository's MIT
license — see `LICENSE` for details.

## License

MIT © 2026 Najeeb Al-Shabibi (code). See `LICENSE`.
