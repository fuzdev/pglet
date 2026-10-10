-- A scalar subquery whose plan is a **set operation** — the shape that gives
-- `Plan::output_columns_scoped`'s own unified-column arm a reader.
--
-- Each shape's descriptors are built twice, once by the executor's node and once by the
-- planner, and until a subquery could carry an enum nothing in the engine read the
-- planner's half at all (`docs/enums.md` recorded it as a second producer kept in step,
-- not a guarded rule). It is read here: the subquery's enum is the planner's unified
-- descriptor's, and a `None` there puts this comparison back on the label bytes, which
-- for `mood` is the opposite verdict.
--
-- The `UNION` (not `UNION ALL`) is what keeps the subquery scalar: both branches yield
-- the one row, and 18.6 raises `more than one row returned by a subquery` otherwise.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
CREATE TABLE u (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
INSERT INTO u VALUES (1, 'happy');

-- @test
SELECT
  (SELECT m FROM u UNION SELECT m FROM u) > 'ok' AS both_branches_enum,
  (SELECT m FROM u UNION SELECT 'happy') > 'ok' AS one_branch_unknown;
