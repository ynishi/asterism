// Prints `corpus.txt` in `Intl.Collator("en")` order, one entry per
// line, for `just collation-jsc` to diff against `golden-icu.txt`.
//
// Written for JavaScriptCore's `jsc` shell (`read` / `print` builtins),
// because JSC is the engine WKWebView runs and therefore the one whose
// ICU actually ships to users. vitest exercises the same golden on Node;
// this is the check that the two agree. Run from this directory.
var corpus = read("corpus.txt").split("\n").filter(function (l) {
  return l.length > 0;
});
corpus.sort(new Intl.Collator("en").compare);
print(corpus.join("\n"));
