-- The virtual catalog relations are matched exactly, like every other relation name.
-- The bare `pg_class` spelling and `PG_CLASS` both resolve (the parser downcased the
-- second); a quoted `"PG_CLASS"` is a relation neither engine has (`42P01`). The old
-- `to_lowercase` in `CatalogTable::from_name` resolved it, and it was the one relation
-- class that folded — an ordinary user table never did, so the two answered
-- differently for the same spelling.
-- @test
SELECT count(*) >= 0 AS present FROM "PG_CLASS";
-- @expect-error: a quoted catalog relation name is exact (42P01)
