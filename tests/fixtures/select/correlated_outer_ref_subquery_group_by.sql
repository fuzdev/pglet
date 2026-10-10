-- The correlated outer reference in the **subquery's own `GROUP BY`**. Every clause of
-- a nested level is re-resolved, not just its `WHERE`: the descent visits the
-- `Plan::Aggregate` node's `group_by` list the same way it visits a `Filter`'s
-- predicate.
-- @setup
CREATE TABLE cg_o (okey INT, ouid UUID);
CREATE TABLE cg_i (ikey INT, itag TEXT);
INSERT INTO cg_o VALUES (1, '11111111-1111-1111-1111-111111111111');
INSERT INTO cg_i VALUES (1, 'a');
-- @test
SELECT okey, (SELECT max(ikey) FROM cg_i WHERE cg_i.ikey = cg_o.okey GROUP BY cg_o.ouid = '11111111-1111-1111-1111-111111111111') AS m FROM cg_o ORDER BY okey;
