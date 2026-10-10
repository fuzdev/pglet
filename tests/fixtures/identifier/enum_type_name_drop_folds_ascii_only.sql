-- `executor::ddl::execute_drop_type` reads back the key `execute_create_type` wrote, so
-- it has to fold at the same ASCII width. `CREATE TYPE ZÄP` registers `zÄp`;
-- `DROP TYPE IF EXISTS zäp` then names nothing and skips (a NOTICE on both engines),
-- `DROP TYPE ZÄP` removes it, and the freed name takes a second `CREATE TYPE` with a
-- different label — which is what the `@test` reads back, so a drop that silently failed
-- shows up as `42710` rather than as a missing row.
-- The `IF EXISTS` spelling is here because it is the drop site's **silent** direction: a
-- skip is a legal outcome, so a lookup that misses reports success. The bare `DROP TYPE`
-- beside it is the loud one, and the pair is what makes this reach the drop site from
-- both sides.
-- No other fixture in the corpus issues a `DROP TYPE` at all, so this one carries that
-- site alone (checked by grep, not asserted). It reddens on a single-site
-- revert to a Unicode `to_lowercase()` at any of the three type-name sites: at
-- `execute_create_type` the `IF EXISTS` drop then *hits* and the bare `DROP TYPE` cannot
-- find what it removed; at `execute_drop_type` neither lookup finds `zÄp`; at
-- `resolve_column_type` the `ZÄP` column type resolves to nothing (`42704`).
-- @setup
CREATE TYPE ZÄP AS ENUM ('a');
DROP TYPE IF EXISTS zäp;
DROP TYPE ZÄP;
CREATE TYPE ZÄP AS ENUM ('b');
CREATE TABLE zt (c ZÄP);
INSERT INTO zt VALUES ('b');

-- @test
SELECT c::text AS c FROM zt;
