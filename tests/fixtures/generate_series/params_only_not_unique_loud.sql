-- Two bare parameters fit three signatures equally: 42725 at `Parse`.
-- @param $1 int4 1
-- @param $2 int4 3
-- @expect-error: function generate_series(unknown, unknown) is not unique
-- @test
SELECT * FROM generate_series($1, $2);
