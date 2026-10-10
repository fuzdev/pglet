-- ...through UPDATE's own table lookup, which is a different call site.
-- @setup
CREATE SEQUENCE sequpd;
-- @test
UPDATE sequpd SET last_value = 5;
-- @expect-error: a sequence is not a writable table (42809)
-- @no-error-fields: 18.6 answers `UPDATE <a sequence>` with `cannot change sequence "x"` and no DETAIL; pglet refuses the relation kind instead and its wording carries one
