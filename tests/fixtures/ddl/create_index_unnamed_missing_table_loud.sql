-- The unnamed spelling keeps the statement's other gates: a table no relation holds is
-- `42P01`, as with a name.
-- @test
CREATE INDEX ON ciu22_nosuch (a);
-- @expect-error: relation "ciu22_nosuch" does not exist (42P01)
