-- `EXISTS(SELECT ...)` in a no-FROM projection — `SELECT EXISTS(...) AS e`. This
-- is the `query_has_fact` shape: the outer SELECT has no FROM (planned to a
-- VALUES node), and the EXISTS projection's inner sub-plan scans a real table.
-- Postgres names an unaliased EXISTS projection `exists`; here it is aliased.
-- @setup
CREATE TABLE u (id INT, flag BOOLEAN);
INSERT INTO u VALUES (1, true), (2, false);
-- @test
SELECT EXISTS (SELECT 1 FROM u WHERE u.flag = true) AS e;
