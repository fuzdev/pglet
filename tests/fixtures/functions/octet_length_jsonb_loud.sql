-- No `octet_length(jsonb)`: 42883.
-- @test
-- @expect-error: function octet_length(jsonb) does not exist
SELECT octet_length('{}'::jsonb);
