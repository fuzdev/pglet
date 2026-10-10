-- The `42703` half of `update/zero_row_set_bogus_qualifier_loud.sql`: a **bare** name no
-- relation in scope carries, in a `SET` right-hand side the statement never evaluates.
-- `UPDATE 0` here, `42703` there.
--
-- The pair is what pins the binding family's split — a qualifier naming no relation is
-- `errorMissingRTE`'s `42P01`, a name no relation carries is `errorMissingColumn`'s
-- `42703` — in a position where neither used to be raised at all.
-- @setup
CREATE TABLE zru (id INT, v TEXT);
INSERT INTO zru VALUES (1, 'a');

-- @expect-error: column "nosuchcol" does not exist (42703)
-- @test
UPDATE zru SET v = nosuchcol WHERE 1 = 0;
