-- Two quantifiers need no rule of their own on either engine: both keywords are
-- reserved, so the second reaches the expression parser and is a syntax error there.
-- Pinned because the change teaches the parser to *eat* one of them, and eating both
-- (a loop rather than a single choice) would accept this. 42601 on both, in either
-- order — the mirror `count(DISTINCT ALL a)` is the same refusal.
-- @setup
CREATE TABLE sq_two (a INT);
INSERT INTO sq_two VALUES (1), (1), (2);
-- @test
-- @expect-error: syntax error at or near "DISTINCT"
SELECT count(ALL DISTINCT a) FROM sq_two;
