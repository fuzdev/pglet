-- The `text` member, which is the one that identifies what the class was.
--
-- Over every other argument type the two declared-type walkers disagreed (the argument's
-- own OID against 25); over `text` the executor walk's fallback is `text` too, so the two
-- legs agreed at 25 and the pair-comparison saw nothing — while 18.6 refuses this
-- spelling exactly as it refuses the others. An agreeing pair is not a conforming one.
-- @setup
CREATE TABLE unnest_na_tx (tx TEXT);
-- @test
-- @expect-error: no unnest(text)
SELECT unnest(tx) FROM unnest_na_tx;
