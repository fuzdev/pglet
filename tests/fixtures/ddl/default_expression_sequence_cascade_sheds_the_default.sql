-- `CASCADE` over that same edge sheds the **whole** default (measured on 18.6:
-- `column_default` reads SQL NULL afterwards) and keeps the column and its rows.
-- @no-declared-types: `information_schema.columns.column_name` is Postgres's `name`
-- type (OID 19); pglet's synthesized view still declares it `text` (25) — `name` exists as a type now but the catalog columns have not moved to it. The same
-- catalog-view type gap the other `information_schema` fixtures waive, and outside
-- this fixture's subject.
-- @setup
CREATE SEQUENCE def_cas_seq;
CREATE TABLE def_cas_t (a INT DEFAULT nextval('def_cas_seq') + 1);
DROP SEQUENCE def_cas_seq CASCADE;
-- @test
SELECT column_name, column_default IS NULL AS shed
FROM information_schema.columns WHERE table_name = 'def_cas_t';
