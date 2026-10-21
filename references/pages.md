# Writing a Spacesheep page people take in

For any document, report, plan or explainer you publish as a Spacesheep page. The rules are best first, each with
its reason and source (researched for Pointman, thread 6914). Thresholds the checker measures are in **bold**.

## How to write one

1. **Start from `template.html`** in this folder: the answer-first top, KPI tiles, a section pattern, a diff with
   numbered notes, folded sources and "Not checked" are already there. Replace its words and data; keep its shape.
2. **Write to the rules below.** The first five matter most.
3. **Check it before every deploy:** `python3 check.py page.html` (this folder's `check.py`; a folder with an
   `index.html` works too). Fix every failure; a warning needs a reason to stay. Content a script adds isn't seen,
   so a diagram drawn with plain elements counts as a visual only with `role="img"` and an `aria-label`.
4. **Look at it at phone width** (320 px) before you post the link: no sideways scroll, nothing only on hover.

## The rules

1. **Answer first.** The first screen (**the first 150 words**) says what this is, the result or decision, and what you need from the reader.
   Why: readers spend 57% of their viewing time above the fold and read about 20% of a page's words. Putting the conclusion first means they keep the main point wherever they stop.
   Source: NN/g, [Scrolling and Attention](https://www.nngroup.com/articles/scrolling-and-attention/) (2018); Nielsen, [How Little Do Users Read?](https://www.nngroup.com/articles/how-little-do-users-read/) (2008); Schade, [Inverted Pyramid](https://www.nngroup.com/articles/inverted-pyramid/) (2018).

2. **Show the problem or the result as a picture** (before and after side by side, or a diagram of what changed) **within the first 150 words**.
   Why: words plus pictures beat words alone, the largest effect in multimedia learning (median d = 1.67). 150 words is about one phone screen.
   Source: Mayer, [Using multimedia for e-learning](https://eric.ed.gov/?id=EJ1153516), J. Computer Assisted Learning 33(5), 2017 (multimedia principle).

3. **Write in the reader's words.** Use one name for each thing, and define a new term in a few words where it first appears. **No code names (functions, flags, paths, hashes) in the lead, headings or KPI labels.** Keep them for diffs, code and folded detail.
   Why: learners do better when they know the key terms first (d = 0.46). Experts also want plain text and give up on jargon. STE allows one word, one meaning.
   Source: Mayer 2017 (pre-training principle); Loranger, [Plain Language Is for Everyone, Even Experts](https://www.nngroup.com/articles/plain-language-experts/) (NN/g 2017); ASD-STE100 Issue 9 (2025), [summary](https://en.wikipedia.org/wiki/Simplified_Technical_English).

4. **Make each heading state its point**, key words first ("Fix 2: letters go last", not "Fix 2" or "Background"). The headings alone should tell the story.
   Why: sentence headlines gave better understanding, fewer misconceptions and better recall a week later than topic titles. Scanners read a heading's first two words, and many read only the headings.
   Source: Garner & Alley, [How the design of presentation slides affects audience comprehension](https://pure.psu.edu/en/publications/how-the-design-of-presentation-slides-affects-audience-comprehens/), Int. J. Eng. Educ. 29(6), 2013 (n = 110); Nielsen, [First 2 Words](https://www.nngroup.com/articles/first-2-words-a-signal-for-scanning/) (2009); Pernice, [F-Shaped Pattern](https://www.nngroup.com/articles/f-shaped-pattern-reading-web-content/) (NN/g 2017); Mayer 2017 (signalling, d = 0.46).

5. **Give each section one point: 1–2 sentences, then its visual.**
   Why: lessons split into parts the reader moves through at their own pace teach better (d = 0.70). Working memory holds about four chunks. Scannable text scored 47% higher in usability.
   Source: Mayer 2017 (segmenting); Cowan, [The magical number 4](https://pubmed.ncbi.nlm.nih.gov/11515286/), Behav. Brain Sci. 24, 2001; Morkes & Nielsen, [Concise, SCANNABLE, and Objective](https://www.nngroup.com/articles/concise-scannable-and-objective-how-to-write-for-the-web/) (1997).

6. **Put each explanation next to what it explains.** Label diagrams directly, not through a legend. Put code notes beside the lines they explain (under them on a phone), numbered to match.
   Why: words placed next to the picture they describe help learning (d = 0.79). The split-attention meta-analysis found d = 0.85.
   Source: Mayer 2017 (spatial contiguity); Ginns, Learning and Instruction 16, 2006, as reported in [Beege et al. 2019](https://www.frontiersin.org/journals/education/articles/10.3389/feduc.2019.00086/full).

7. **Cut what doesn't serve the reader's task:** decorative images, history, asides and "interesting" detail. Order sections by what the reader needs. The third screen onwards gets under a fifth of their time.
   Why: leaving out extra material helps learning (d = 0.70). In a meta-analysis of 58 studies, interesting but irrelevant details lowered scores, most of all next to a relevant diagram. Concise text scored 58% higher. Readers ignore decorative images.
   Source: Mayer 2017 (coherence); Sundar & Adesope, Educ. Psychol. Rev. 2020 ([WSU summary](https://news.wsu.edu/press-release/2020/03/19/seductive-details-inhibit-learning)); Morkes & Nielsen 1997; Nielsen, [Photos as Web Content](https://www.nngroup.com/articles/photos-as-web-content/) (2010); NN/g 2018 (81% of time in the first three screens).

8. **Keep sentences and paragraphs short.** Write **sentences of at most 25 words (20 in a step)**, in the active voice, one idea each, and **paragraphs of at most 60 words**.
   Why: STE caps descriptive sentences at 25 words and instructions at 20. GOV.UK calls sentences over 25 words difficult. Readers follow over 90% of text that averages 14 words a sentence.
   Source: ASD-STE100 Issue 9; GDS, [Sentence length: why 25 words is our limit](https://insidegovuk.blog.gov.uk/2014/08/04/sentence-length-why-25-words-is-our-limit/) (2014, citing Wylie).

9. **Put a visual at least every 120 words of running text.** A list item over 25 words, or a table cell over 10, counts as running text, not as a visual.
   Why: about two-thirds of a phone screen, so no screen is only text. Scannable text scored 47% higher, and readers get about 20% of the words.
   Source: Morkes & Nielsen 1997; Nielsen 2008.

10. **Pick the visual by the question.** Show before and after side by side. Show how the parts connect as a diagram, and a trend as a chart. Put steps in a numbered list and options in a table. Show a code change as a diff. **Table cells: at most 10 words, no full sentences. At most 5 options side by side.**
    Why: performance is best when the format fits the task: tables for exact values, charts for patterns. Comparison tables work up to five items, and cells should avoid full sentences. STE puts one instruction in each step.
    Source: Vessey, Cognitive fit, Decision Sciences 22(2), 1991 ([summary](https://is.theorizeit.org/wiki/Cognitive_fit_theory)); Moran & Dykes, [Comparison Tables](https://www.nngroup.com/articles/comparison-tables/) (NN/g 2024); ASD-STE100 Issue 9.

11. **Show the numbers that matter as KPI tiles (at most 4) and the rest in one table.**
    Why: about four chunks fit in working memory, and tiles signal which numbers matter (signalling, d = 0.46).
    Source: Cowan 2001; Mayer 2017.

12. **Colour only what changed or what decides, and pair the colour with a sign** (+/−, ↑/↓ or a word).
    Why: one item in a unique colour stands out at a glance, but if many are coloured, none stand out. Colour alone fails readers with poor colour vision.
    Source: Healey, [Perception in Visualization](https://www.csc2.ncsu.edu/faculty/healey/PP/) (NCSU); W3C, [WCAG 1.4.1 Use of Color](https://www.w3.org/WAI/WCAG22/Understanding/use-of-color.html).

13. **Fold detail, raw output and sources** in `<details>`, with a summary that says what's inside. Don't fold what most readers need. Put the limits as a short unfolded list at the end.
    Why: showing the core first and the rest on request makes a page easier to learn. Hidden content gets ignored, so fold only what few readers need and label it well.
    Source: Nielsen, [Progressive Disclosure](https://www.nngroup.com/articles/progressive-disclosure/) (2006); Loranger, [Accordions for complex content](https://www.nngroup.com/articles/accordions-complex-content/) (NN/g 2014).

14. **Keep the flow free of exits and motion.** In running text, link only what the reader must open now (the task, the PR), and link the key words, never "here". Put other links under Sources. **Nothing moves, plays or pops up on its own.**
    Why: each link is a decision that adds load and slows reading. Skimmers read the linked words and skip the rest. Something that starts to move grabs attention whether the reader wants it to or not.
    Source: DeStefano & LeFevre, Cognitive load in hypertext reading, Computers in Human Behavior 23(3), 2007; Fitzsimmons et al., [Skim reading and hyperlinks](https://journals.plos.org/plosone/article?id=10.1371/journal.pone.0239134), PLOS ONE 2020; Abrams & Christ, [Motion onset captures attention](https://pubmed.ncbi.nlm.nih.gov/12930472/), Psych. Sci. 2003; W3C, [WCAG 2.2.2 Pause, Stop, Hide](https://www.w3.org/WAI/WCAG22/Understanding/pause-stop-hide.html).

15. **Make it readable on a phone.** **At 320 px wide, the page has no sideways scroll** (wide tables, code and diagrams scroll in their own box). **Tap targets are at least 44 px**, and **lines of text at most about 75 characters**. Anything that shows on hover also shows on tap.
    Why: WCAG's reflow width is 320 px, and its larger target size is 44 px. Lines of 50–75 characters are easiest to read. Hard text takes longer on a phone.
    Source: W3C, [WCAG 1.4.10 Reflow](https://www.w3.org/WAI/WCAG22/Understanding/reflow.html) and [2.5.5 Target Size](https://www.w3.org/WAI/WCAG22/Understanding/target-size-enhanced.html); Moran, [How Chunking Helps Content Processing](https://www.nngroup.com/articles/chunking/) and [Reading Content on Mobile Devices](https://www.nngroup.com/articles/mobile-content/) (NN/g 2016).

## Why these limits

The 150-, 120-, 60- and 25-word limits are house numbers set by a phone screen, not by a study. They were measured on
three pages (`check.py` reads the same things):


Script: `tools/metrics.py` (rough; copies of the pages are beside it). Folded content excluded.

| | Search box: two fixes (easy) | AGC-16, version from 02:49 (hard) | AGC-16, rebuilt 03:35 |
|---|---|---|---|
| Table cells over 10 words | 0 (longest 7) | 14 (longest 35) | 0 (longest 6) |
| List items over 25 words | 1 | 6 (longest 45) | 0 |
| Code names outside folds | 14 | 87 | 4 |
| Longest run of text between visuals (long list items and cells count as text) | 105 | 949 | 54 |
| Words before the first visual | 42 | 39 | 54 |

Counting words outside tables and lists doesn't catch the hard page: its text sat in tables and lists. Rule 9's long items, and the cell and code-name limits, do.
