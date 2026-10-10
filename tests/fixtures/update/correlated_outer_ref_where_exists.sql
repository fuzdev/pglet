-- The same correlated-outer-reference fix at an `UPDATE`'s `WHERE`. `lower_update`
-- builds the target relation's scope after the subquery is already lowered, exactly as
-- `lower_select` does, so it runs the same re-resolution pass — without it the literal
-- stayed text and the statement failed `42883` on a shape Postgres updates.
-- @setup
CREATE TABLE cu_o (okey INT, ouid UUID, note TEXT);
CREATE TABLE cu_i (ikey INT);
INSERT INTO cu_o VALUES (1, '11111111-1111-1111-1111-111111111111', 'before'), (2, '22222222-2222-2222-2222-222222222222', 'before');
INSERT INTO cu_i VALUES (1), (2);
-- @test
UPDATE cu_o SET note = 'after' WHERE EXISTS (SELECT 1 FROM cu_i WHERE cu_i.ikey = cu_o.okey AND cu_o.ouid = '11111111-1111-1111-1111-111111111111') RETURNING okey, note;
