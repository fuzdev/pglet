-- ...and a table that does not exist is 42P01.
-- @test
CREATE SEQUENCE own_bad_tbl OWNED BY nosuch_owner_table.a;
-- @expect-error: relation does not exist (42P01)
