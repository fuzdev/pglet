-- A `[]` makes the modifier the **element's**, so the refusal is the element's too and
-- names `char` rather than an array-flavoured spelling. 18.6 reaches it the same way:
-- `LookupTypeName` resolves `char(0)[]` to `_bpchar` before `typenameTypeMod` reads the
-- modifier, and `_bpchar.typmodin` is `bpchartypmodin`.
-- @expect-error: a character length below 1 under an array suffix
-- @expect-message
-- @test
SELECT '{ab}'::char(0)[] AS never_reached;
