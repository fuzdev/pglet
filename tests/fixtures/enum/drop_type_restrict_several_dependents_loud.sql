-- `RESTRICT` is the bare spelling: with several dependents it is still `2BP01`, nothing
-- dropped. (18.6 lists every dependent in the detail in an order its OIDs decide; pglet
-- omits the detail when there is more than one.)
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE mr (a TEXT CHECK (a::mood IS NOT NULL), b TEXT DEFAULT ('ok'::text)::mood);

-- @test
-- @expect-error: two dependents, no CASCADE
-- @expect-message
DROP TYPE mood RESTRICT;
