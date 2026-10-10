-- Under an alias a bare `generate_series` names nothing.
-- @expect-error: column generate_series does not exist
-- @test
SELECT generate_series FROM generate_series(1, 2) g;
