-- A `DELETE` whose predicate reads a sub-plan projecting the target's column over a
-- one-column inner table that lacks the name: 18.6 binds the reference to the target
-- row and deletes row `1`; a sub-plan-level binding check refused it `42703`.
-- @setup
CREATE TABLE ctd_p (a INT, b INT);
CREATE TABLE ctd_q (c INT);
INSERT INTO ctd_p VALUES (1, 10), (2, 20);
INSERT INTO ctd_q VALUES (5);
-- @test
DELETE FROM ctd_p WHERE (SELECT ctd_p.b FROM ctd_q) = 10 RETURNING a;
