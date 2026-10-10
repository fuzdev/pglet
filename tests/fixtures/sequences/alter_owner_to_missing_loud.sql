-- A tolerated tail (`OWNER TO`) still resolves its target: pglet models no roles, so
-- the clause does nothing, but naming no relation is 42P01 on both engines.
-- @test
ALTER SEQUENCE no_such_sequence_owner OWNER TO postgres;
-- @expect-error: relation does not exist (42P01)
