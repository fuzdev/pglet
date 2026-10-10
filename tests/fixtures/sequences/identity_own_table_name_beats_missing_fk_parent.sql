-- Same placement, against the other constraint kind: the foreign keys are added after
-- the heap too, so the missing parent's 42P01 never runs.
-- @test
CREATE TABLE idownfk (x INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME idownfk), y INT REFERENCES idownfk_no_such_parent(id));
-- @expect-error: the table's own name is taken before its foreign keys resolve (42P07)
