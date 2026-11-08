# A research agent

A harness that reads the web and writes its owner a report, proved unable to send anything a page says anywhere without asking. It follows CaMeL's split: the model that plans never sees a page.

- [research.pata](research.pata) is the harness. Each round, the planner (`brain.choose`) chooses a URL to fetch, or answers. The page goes to the reader (`reader.extract`), and the reader's note goes to the report (`report.write`). The planner is told only which URL was read.
- The `.json` files are its capabilities' contracts:
  - `net.fetch`'s result is labelled `web`. Its precondition is `label(args.url) == [] or contains(grant.netAllow, args.url)`: a URL that depends on nothing untrusted is fetched as it is, and any other needs to be allowed first.
  - `net.ask` allows a URL, as in [T1](../claude-code/README.md).
  - `brain.choose` and `reader.extract` have no `ret`, so what each says is labelled with what it read. The reader's note is labelled `web`, while the planner's choices stay trusted.
  - `report.write` has no precondition: the report goes to the owner, and a page's content may go there.

## Running it

```console
$ node javascript/cli.js run examples/research --input "Research https://example.com/ for me."
task: Research https://example.com/ for me.
model: chooses net.fetch
net.fetch: https://example.com/
reader: reads the page
report: The page is titled "Example Domain".
model: answers
{"kind":"halted","value":"What I found is in the report."}
```

The planner chose the URL from the task, so nothing asks. The answer has no label: nothing the planner heard came from the page. [javascript/test/research.test.js](../../javascript/test/research.test.js) runs it with a page that says to fetch another URL, which never reaches the planner. It also runs a harness that fetches the reader's note, and that fetch is refused at the call.

## The claim

**A call to `net.fetch` whose URL depends on something untrusted can't happen until a person has allowed that URL through `net.ask`.** It's a rule with a `when`:

```text
net.fetch, when len(label(args.url)) != 0,
  approved by net.ask when result == "allow" and approval.url == args.url
```

[proofs/research.test.mjs](../../proofs/research.test.mjs) has it in kernel JSON. The rule is over the contracts, so it holds for every harness on this interface and whatever either model says, not only for `research.pata`. A harness that follows links from the page would have to ask before each one. cvc5 proves the rule's queries and Ethos checks the proofs, and Lean proves its statements and Comparator checks them. With `net.fetch` ungated, there's a counterexample.

That the planner never sees a page is how `research.pata` is written, not part of the claim. It's what keeps its asks rare. A harness that hands a page to the planner is still safe, but every URL it chooses after that asks.

## What the claim rests on

- **Every `ret` is honest.** `net.fetch`'s result really is labelled `web`. Without that `ret`, the rule still proves, but a page would come back trusted, so the rule would say nothing about it. Anything that stores a value and gives it back must label it too. That's why the report is write-only.
- **`net.fetch` is the only way out.** Nothing checks that. `report.write` goes to the owner.
- **The planner isn't a source.** `brain.choose` has no `ret`: what it says is taken to depend only on what it's shown. A model that is itself hostile isn't what this guards against.
- **A task the owner typed is trusted.** The host labels the start. A task forwarded from someone else must be labelled as theirs.
- What T1's rules rest on: a compliant interpreter, `net.ask`'s answer being a person's, the encoder, and the checker. See [Rules](../../docs/2026-11-05-rules.md).
