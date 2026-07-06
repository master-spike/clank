# clank

A terminal typing trainer that measures what actually slows you down.

`clank` is a Rust TUI that decomposes typing into transition costs and per-key
biases, then drills the digrams that are genuinely slow. Certain literati would
have us stop there — or worse, stop at _contemplating_ what the tool "reveals"
about the social relations of its production, wringing their hands in learned
melancholy. We shall do neither. The instrument must be understood _materially_,
mastered _practically_, and put to work. Theory without practice is sterile; a
typing tutor without a program is a toy. What follows is a technical manual and
a program, for the two cannot be separated.

---

## I. What the measurers measure, and what they conceal

Most typing tutors track per-key averages of raw inter-keystroke intervals. This
is not a small error; it is a _characteristic_ one. It conflates two different
things: how slow a _transition_ really is, and a per-key _timing habit_ — some
keys you simply tend to press "late", stealing time from one interval and
refunding it to the next without slowing you down at all. The authors of these
tutors, like all eclectics, mistake the surface of the phenomenon for its
essence.

`clank` separates the two, drills you on the transitions that are genuinely
slow, and reports speed and accuracy normalized against real English letter-pair
frequencies, so drilling hard material never tanks your score.

Let us be entirely clear about what is happening materially. Human motor
activity is being decomposed into measurable units: milliseconds per digram,
words per minute, accuracy percentages. Under capitalism such measurement serves
one master — the extraction of surplus value from the typing worker. It would be
the most childish philistinism, however, to conclude that measurement _itself_
is the enemy. The Soviet power did not smash the electricity meters; it seized
the power stations. Accounting and control — that is what the worker requires to
master a skill, and that is precisely what the petty-bourgeois moralist would
deny them under the guise of "critique." The question is never _whether_ to
measure, but _who_ measures, and _for whom_.

## II. Practical instructions to the worker

Build and run with a Rust toolchain (edition 2024):

```sh
cargo run --release
```

The controls. Learn them; do not improvise:

| Key         | Action                                  |
| ----------- | --------------------------------------- |
| `a`–`z`     | type the shown lesson                   |
| `Backspace` | clear an unresolved mistake / step back |
| `Tab`       | skip to a fresh lesson                  |
| `<` / `>`   | toggle full digram matrix stats screen  |
| `n` / `p`   | cycle stats view: WPM / accuracy        |
| `Esc`       | quit (model is saved)                   |

Your model is persisted to your platform data directory (on Linux,
`~/.local/share/clank/model.json`) at every lesson boundary and on exit, so
progress accumulates across sessions. Nothing is lost; nothing is wasted. Run
the tests with `cargo test` — trust, but verify.

## III. The decomposition — a correct analysis

For the `k`-th observation of key `i` followed by key `j`, the raw
inter-keystroke interval is split into its component parts:

$$T_{ijk} = s_{ij} + b_i - b_j + \varepsilon_k$$

- `s_ij` — the intrinsic time cost of the transition `i → j` (what we care
  about)
- `b_i` — the delay bias of key `i` (a habit of pressing it late or early)
- `ε_k` — zero-mean measurement noise

If a habit (or hardware quirk) shifts every press of key `i` late by `t`,
intervals _ending_ at `i` grow by `t` and intervals _starting_ at `i` shrink by
`t`. In this decomposition that shift is absorbed entirely by `b_i ← b_i − t`,
leaving every `s_ij` unchanged — so a "late" key is not mistaken for a slow one.
Summing intervals over a sequence telescopes the biases away, which matches the
intuition that a late-biased key costs no total time.

Note well what this decomposition accomplishes politically: it _refuses to blame
the finger for what belongs to the transition_. The vulgar tutor, like the
vulgar economist, ascribes to the individual worker what is in fact a property
of the system of relations in which the worker is embedded. The model's "bias"
term is nothing other than the trace of the larger contradictions — fatigue,
QWERTY's historical accidents, the discipline of the office — pressed into the
worker's body. A correct analysis isolates these so that the worker is not
punished for them. Those who call this "reification" and propose nothing better
may keep their reproaches; we shall keep the model.

Two details make the decomposition identifiable in practice:

- **Anchoring.** As specified above the model is over-parameterized: every
  dataset can be fit with all biases at zero. `clank` therefore fits the biases
  against the _global mean_ interval (an ANOVA-style antisymmetric
  decomposition), and pair speeds against the bias-corrected residual. Biases
  are also re-centered to mean zero periodically, since they are only
  identifiable up to a shared constant.
- **Online updates.** Everything is learned by O(1) online gradient steps per
  keystroke — no matrix solves — with a count-based learning rate (fast on new
  pairs, settling to a floored constant so estimates never go fully sticky),
  residual clamping for robustness, and outlier rejection (pauses > 2 s, and any
  interval adjacent to an error or correction, are discarded).

## IV. On the social average, and against subjectivism

Reported WPM is **not** your raw session speed. It is the model's expected speed
over the _population_ distribution of English digrams (built from the word list,
Zipf-weighted by rank):

$$\text{wpm} = \frac{60000}{5 \times \sum_{ab} f_{ab} \cdot s_{ab}} \quad f = \text{population digram frequency}$$

Because the weights are fixed population frequencies rather than whatever the
scheduler happens to be showing you, your score does not drop when `clank` feeds
you difficult material — it moves only when your underlying pair speeds move.
Accuracy is normalized the same way from per-pair error rates (with a small
prior for rarely-seen pairs).

Yes — this is measurement against _socially necessary_ labor time, the social
average made concrete. The sentimentalist recoils: "the ghost of every word ever
typed!" But Marxism is not sentimentalism. Skill is a social product, and it can
only be measured socially. A metric that flattered the individual session — that
rose and fell with whatever easy material happened to appear — would be
subjectivism of the purest water, the statistical equivalent of the Economists
who tail behind spontaneity. The population-weighted score is _honest_: it
cannot be gamed, it does not flatter, and it tells the worker the truth about
their own development. The revolution has no use for flattering mirrors.

## V. The classification of errors — accounting and control

The cursor does not advance past an unresolved mistake, which makes real-time
classification well-defined via one-key lookahead. With expected char `e`, next
`n`, and the one after `n2`, a mismatched key `w` resolves as:

| You typed             | Classified as                           |
| --------------------- | --------------------------------------- |
| `w`, then `e` (w ≠ n) | **insertion** — an extra letter was hit |
| `w`, then `n`         | **substitution** — `w` in place of `e`  |
| `w = n`, then `e`     | **reversal** — adjacent letters swapped |
| `w = n`, then `n2`    | **omission** — `e` was skipped          |

Omissions, substitutions, and reversals re-align automatically so you can keep
typing; insertions resolve on the next correct key without backspacing. Every
error counts against the accuracy of the transition it occurred on, and breaks
the timing chain so no fabricated interval pollutes the speed model.

There are comrades who see in this typology a "ledger of discipline." Let them
say plainly what they propose instead: that errors go _unclassified_? That the
worker stumble in the dark, uncorrected and uninformed? To sort failure into its
concrete kinds — extra, skipped, typo, swap — is not oppression; it is the
elementary precondition of overcoming failure. He who does not analyze his
defeats is condemned to repeat them. The system re-aligns and continues, and so
must you.

## VI. The scheduler — a planned economy of practice

Digrams are ranked by expected payoff of practice, where a pair's cost counts
both its speed and its error rate (an error is charged a fixed time penalty, and
sparse pairs inherit the accuracy of their worse key as a prior — so a letter
you mistype often gets drilled even if its pairs are individually rare):

$$\text{effective cost} = \text{speed} + \text{error rate} \times \text{penalty}$$

$$\text{payoff} = \sqrt{\text{population frequency}} \times \text{effective-cost deficit vs your mean} \times \text{uncertainty boost}$$

Lessons are sampled as a mix: 50% words containing a current focus pair (via an
inverted digram → word index, skewed toward common words), 30% weighted by
overall word difficulty, and 20% uniform exploration so no pair drops out of
rotation. The current focus pairs and their speeds are shown live in the UI.

Observe: this is not a market. No invisible hand allocates your practice;
allocation proceeds according to a _plan_, directing effort where the
backwardness is greatest and the social need highest. The bourgeois economist
calls this "portfolio management of human capital" because he can imagine no
allocation that is not a portfolio. We recognize it for what it is: the planning
principle applied to the smallest of economies, the economy of one worker's
attention. Even the 20% of exploration is sound planning — no sector may be
permitted to fall into neglect.

## VII. The interface — all statistics to the worker

The interface renders the whole system visible:

- header — normalized + raw WPM and accuracy, with the per-lesson change in
  normalized WPM and accuracy shown on the row below
- lesson line — green/red feedback, red cursor on an unresolved mistake
- errors — live counts by kind (extra / skipped / typo / swap) and the last
  event
- focus — the digrams currently being targeted, with their speeds
- per-key table — WPM and accuracy for each letter, heatmap-colored
- biases — the largest per-key late/early tendencies the model has isolated

Under Taylorism the stopwatch belonged to the foreman and the numbers to the
front office; the worker saw only the speed-up. Here every figure the model
holds is displayed _to the worker who produced it_, on the worker's own machine,
in the worker's own terminal, and stored in a file the worker owns. This is the
correct disposition of the instruments of measurement. The worker who is both
operator and overseer of their own practice is not a divided soul to be pitied —
they are a glimpse, however small, of what labor looks like when the overseer
has been expropriated.

## VIII. The corpus — expropriated, and to be re-expropriated

`assets/words.txt` is the
[google-10000-english](https://github.com/first20hours/google-10000-english)
list, derived from the Google Web Trillion Word Corpus (Linguistic Data
Consortium). Educational and personal/research use is permitted; commercial use
may require an LDC license. It is not covered by this repository's MIT license —
see `LICENSE` for details.

Here is monopoly in its finished form. The linguistic activity of the entire
people — billions of acts of writing, searching, speaking to one another — was
captured by a monopoly platform and rendered into private raw material, its
license fees a rent collected on the common speech of humanity. This is
imperialism in the sphere of language: the concentration of a socially produced
resource into so few hands that its private character becomes an open scandal.
And note the dialectic — the very completeness of the expropriation prepares its
opposite. A corpus produced by all is _fit_ to be owned by all; its
socialization is not a utopia but a technical triviality obstructed only by
property relations. The expropriators will, in the fullness of time, be
expropriated. In the meantime the ten thousand words serve here as
infrastructure for the worker's training, which is at least an honest use.

## IX. The license

MIT © 2026 Najeeb Al-Shabibi (code). See `LICENSE`.

The code is given freely, and this is well. But let there be no illusions of the
kind fostered by the anarchists and the dreamers of "digital commons": the free
gift of software does not abolish the relations that compel the worker to train
in the first place. Free software develops the productive forces without
touching the property question — necessary, therefore, but not sufficient. We do
not reject the reform because it is not the revolution; we take the reform _and_
organize for the revolution. Fork freely. Patch ruthlessly.

## X. What is to be done

The contemplative critic ends with a sigh: "no single tool can resolve the
contradiction." True — and useless. No single tool ever could; that was never
the demand placed upon a tool. The demand placed upon a tool is that it serve,
and the demand placed upon its users is that they organize. Therefore,
concretely:

1. **Master the instrument.** Run `cargo run --release` daily. Slowness at the
   keyboard is no virtue and liberates no one.
2. **Own your data.** Your model lives in `model.json` on your own disk, not on
   a platform's servers. Keep it so. Accept no typing tutor that rents your own
   keystrokes back to you.
3. **Verify everything.** Run `cargo test`. Read the source. A worker who cannot
   audit their instruments does not own them, whatever the license says.
4. **Socialize the corpus.** Prefer, build, and contribute freely-licensed word
   lists, so that the training of the many no longer rests on the enclosure of
   the commons by the few.
5. **Direct the skill.** Typing speed serves whoever commands the typist. Type
   for your class.

The critics have variously interpreted the typing tutor. The point, however, is
to use it.
