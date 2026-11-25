-- An argument name is an identifier: `"Hours"` is not `hours`.
-- @expect-error: no make_interval("Hours" => integer)
-- @test
SELECT make_interval("Hours" => 2);
