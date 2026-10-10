-- The correlated-outer-reference fix at a `DELETE`'s `WHERE` — `lower_delete`'s target
-- scope is built after the subquery is lowered, so it runs the same re-resolution pass
-- `lower_select` and `lower_update` do.
-- @setup
CREATE TABLE cd_o (okey INT, ouid UUID);
CREATE TABLE cd_i (ikey INT);
INSERT INTO cd_o VALUES (1, '11111111-1111-1111-1111-111111111111'), (2, '22222222-2222-2222-2222-222222222222');
INSERT INTO cd_i VALUES (1), (2);
-- @test
DELETE FROM cd_o WHERE EXISTS (SELECT 1 FROM cd_i WHERE cd_i.ikey = cd_o.okey AND cd_o.ouid = '11111111-1111-1111-1111-111111111111') RETURNING okey;
