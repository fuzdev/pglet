-- Two CHECK constraints declared with the same name on one table is 42710 as well.
-- Postgres attaches the CHECKs as the heap is created, so it words this one `check
-- constraint "dup" already exists`; pglet has one duplicate-constraint error and
-- uses the relation-scoped spelling. Only the SQLSTATE is compared.
-- @expect-error: check constraint dup already exists
-- @test
CREATE TABLE ctdc (a INT, CONSTRAINT dup CHECK (a > 0), CONSTRAINT dup CHECK (a < 10));
