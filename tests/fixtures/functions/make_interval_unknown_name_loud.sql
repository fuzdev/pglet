-- A name `make_interval` does not have matches no overload (`42883`).
-- @expect-error: no make_interval(nope => integer)
-- @test
SELECT make_interval(nope => 2);
