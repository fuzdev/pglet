-- A set operation inside a scalar subquery whose left side projects an enclosing `INT`
-- column, over an inner table carrying a same-named `text` column. The column is the
-- outer integer, so the right side's `'x'` is 18.6's `22P02`; unified with the inner
-- `text` column it answered.
-- @setup
CREATE TABLE cts_te (k INT);
CREATE TABLE cts_tt (m TEXT, k TEXT);
INSERT INTO cts_te VALUES (1), (2);
INSERT INTO cts_tt VALUES ('x', 'y');
-- @expect-error: a literal side read as the enclosing integer column — 22P02
-- @test
SELECT k, (SELECT o.k FROM cts_tt UNION SELECT 'x' LIMIT 1) AS v FROM cts_te o ORDER BY k;
