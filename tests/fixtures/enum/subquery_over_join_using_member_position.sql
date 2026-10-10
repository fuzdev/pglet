-- The `USING`-merge half of the same producer pair. A scalar subquery over a
-- `JOIN … USING (m)` reads the **planner's** merged descriptor, not the executor's, so
-- this is the other arm `Plan::output_columns_scoped` builds and nothing used to read.
--
-- The join yields exactly one row (`u` holds one), which is what keeps the subquery
-- scalar; `happy` is `mood`'s last member and its first label, so a label-byte answer is
-- `f` and a member-position answer is `t`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
CREATE TABLE u (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
INSERT INTO u VALUES (1, 'happy');

-- @test
SELECT
  (SELECT m FROM t a JOIN u b USING (m)) > 'ok' AS merged,
  (SELECT x FROM (SELECT m AS x FROM t a JOIN u b USING (m)) z) > 'ok' AS merged_through_a_derived_table;
