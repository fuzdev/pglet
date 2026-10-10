-- Documentary: over a table that exists, `DROP CONSTRAINT` is pglet's `0A000`, a loud
-- refusal where 18.6 performs the drop (here, with the constraint absent, it notices and
-- skips). It was a silent no-op under `IF EXISTS`. 18.6 accepts, so any pglet error
-- passes; the record is the refusal, not its code.
-- @setup
CREATE TABLE atie20 (a INT);
-- @expect-error: pglet does not drop constraints
-- @test
ALTER TABLE IF EXISTS ONLY atie20 DROP CONSTRAINT IF EXISTS atie20_pkey;
