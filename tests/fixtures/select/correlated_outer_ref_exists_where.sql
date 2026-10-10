-- The general correlated-outer-reference shape, with **zero column-name overlap**
-- between the outer and inner relations — so nothing about it depends on shadowing.
-- A bare literal compared against an enclosing query's column in a correlated
-- `EXISTS`'s `WHERE` got no coercing cast, because the subquery is parsed *and fully
-- lowered* before the enclosing query's `ColumnScope` exists. The literal stayed text
-- and the eval-time operand gate raised `42883` on `uuid = text`, a query Postgres
-- runs.
--
-- The enclosing level re-resolves the sub-plan against a chained scope afterwards
-- (`planner/correlation.rs`), so `cw_o.ouid` types as `uuid` and the literal is cast.
-- @setup
CREATE TABLE cw_o (okey INT, ouid UUID);
CREATE TABLE cw_i (ikey INT, itag TEXT);
INSERT INTO cw_o VALUES (1, '11111111-1111-1111-1111-111111111111'), (2, '22222222-2222-2222-2222-222222222222');
INSERT INTO cw_i VALUES (1, 'a'), (2, 'b');
-- @test
SELECT okey FROM cw_o WHERE EXISTS (SELECT 1 FROM cw_i WHERE cw_i.ikey = cw_o.okey AND cw_o.ouid = '11111111-1111-1111-1111-111111111111') ORDER BY okey;
