-- A sub-plan's unbound name in the target list does not outrank the enclosing level's
-- missing relation: 18.6 transforms the `FROM` first and answers `42P01`.
-- @setup
CREATE TABLE ctr_q (c INT);
-- @expect-error: the FROM is analyzed before the target list — 42P01
-- @test
SELECT (SELECT bogus FROM ctr_q) FROM ctr_nosuch;
