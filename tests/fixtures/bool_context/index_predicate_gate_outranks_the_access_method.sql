-- The same placement against the GIN method check, which pglet raises from
-- `DefineIndex`'s position and 18.6 from `ResolveOpClass` — both below parse
-- analysis, so the predicate wins on both.
-- @setup
CREATE TABLE bcix3 (a INT);
-- @expect-error: argument of WHERE must be type boolean, not the GIN method refusal
-- @no-profile-lock: a DDL-time refusal; no index is built and no plan is chosen.
-- @test
CREATE INDEX bcix3_ix ON bcix3 USING gin (a) WHERE a;
