-- pglet-stricter, and loud: 18.6 resolves an unknown string literal to `oid` here and
-- answers NULL, while pglet types a string literal `text` (`EDGES.md` row 54) and refuses
-- it `42883`. Documentary — 18.6 accepts, so any pglet error passes; what it holds is that
-- the refusal stays a refusal rather than a silent answer over an unchecked `text`.
-- @test
-- @expect-error: col_description(text, integer) does not exist here
SELECT col_description('1', 1);
