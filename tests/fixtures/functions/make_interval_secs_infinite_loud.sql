-- An infinite `secs` is `22008`.
-- @expect-error: interval out of range
-- @test
SELECT make_interval(secs => 'infinity');
