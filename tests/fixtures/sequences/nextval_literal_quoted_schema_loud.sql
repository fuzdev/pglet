-- A schema qualifier is folded only when the source wrote it **unquoted**. Postgres
-- downcases an unquoted name part and takes a quoted one verbatim, so `"PUBLIC"` names
-- a schema that does not exist and the literal resolves nothing — `42P01` on both.
-- Reading the literal the way `SplitIdentifierString` does — unquoting each part —
-- is what made this silent for a time: the unsplit literal used to carry its quote
-- characters into the `public` comparison and fail it by *accident*, and unquoting
-- removed the accident along with the defect it was hiding. The accept-side twin is
-- `nextval_literal_schema_qualifier_case.sql`, and it needs the *unquoted* spelling,
-- so one fixture cannot carry both polarities.
-- @setup
CREATE SEQUENCE s;
-- @test
SELECT nextval('"PUBLIC".s');
-- @expect-error: a quoted schema qualifier names no schema (42P01)
