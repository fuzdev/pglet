-- A set op's trailing `ORDER BY` keeps its ordinal as a literal so the Sort resolves it
-- by *index* (a set op's unnamed `?column?` columns collide by name), which is a second
-- reader of the same folded constant — and it was silent on `-1` for the same reason the
-- plain clause was. 18.6: `42P10`, and `set_op_order_by_position_out_of_range.sql` is the
-- positive-ordinal sibling.
-- @setup
CREATE TABLE obsa (x INT);
CREATE TABLE obsb (x INT);
INSERT INTO obsa VALUES (1), (2);
INSERT INTO obsb VALUES (3);
-- @test
-- @expect-error: ORDER BY position -1 is not in select list (42P10)
SELECT x FROM obsa UNION ALL SELECT x FROM obsb ORDER BY -1;
