-- With `TO` or `=`, `SET NAMES` is not the `NAMES` form but `generic_set` naming a setting called
-- `names`, which 18.6 does not have: its `42704 unrecognized configuration parameter "names"`.
-- @test
-- @expect-error: unrecognized configuration parameter "names"
SET NAMES TO 'utf8';
