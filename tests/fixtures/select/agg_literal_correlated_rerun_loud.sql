-- An aggregate whose argument names an enclosing level's column is not certain at its own level, and the
-- correlated re-run, which reads the first run's casts, does not read it as one: at the `integer` that
-- re-run reads this would answer `t` where 18.6 answers `f`.
-- @setup
CREATE TABLE aln (g INT, n INT, f8 FLOAT8);
INSERT INTO aln VALUES (1, NULL, 3.5), (1, 2, 1.5), (2, 7, 7.5);
-- @test
-- @expect-error: the literal stays text beside an aggregate over an enclosing column
SELECT (SELECT max(COALESCE(n.n, n.f8 + o.y)) = '4' FROM aln n WHERE g = 1) FROM (VALUES (0.25)) o(y);
