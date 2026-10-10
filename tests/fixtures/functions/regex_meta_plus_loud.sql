-- One regex metacharacter after the anchor, and no other: refused `0A000` here (`EDGES.md`
-- row 251), a valid pattern 18.6 answers. Documentary — any pglet error passes; what it
-- holds is that this character stays in the refused set, since answering the pattern as a
-- literal prefix would be a silent wrong boolean.
-- @test
-- @expect-error: only an anchored literal prefix is supported
SELECT 'aa' ~ '^a+';
