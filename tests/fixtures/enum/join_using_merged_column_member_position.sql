-- `JOIN … USING (m)` emits one merged `COALESCE(l.m, r.m)` column, and that descriptor
-- is built **fresh** in two places that never read each other — `resolve_using` in the
-- executor and `Plan::output_columns_scoped`'s `Join` arm in the planner. Both dropped
-- the enum, so an order over the *merged* name went by label bytes while the same join
-- spelled `ON t.m = u.m` — where each side keeps its own descriptor — went by member
-- position. `join_on_condition_member_position` and `join_order_by_member_position` are
-- that control, and both were green throughout.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
CREATE TABLE u (m mood NOT NULL, tag TEXT);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
INSERT INTO u VALUES ('sad', 'a'), ('ok', 'b'), ('happy', 'c');

-- @test
SELECT t.id, u.tag FROM t JOIN u USING (m) ORDER BY m;
