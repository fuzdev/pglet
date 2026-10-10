-- Postgres resolves `'x'::regclass` at parse analysis but only reaches
-- `init_sequence`'s kind check inside the function body, so `setval`'s arguments are
-- evaluated *between* the two: this is 22012, not the 42809 the same name answers with
-- a well-formed argument. A single kind gate hoisted above the whole call answers this
-- one wrong — which is what pglet did, briefly, in the commit that added the gate.
-- @setup
CREATE TABLE svabk_host (a INT);
-- @test
SELECT setval('svabk_host', 1/0);
-- @expect-error: the argument error outranks the wrong kind (22012)
