-- `transformColumnDefinition` resolves each column's type in the `tableElts` walk,
-- which likewise precedes the before-list — so the unknown type wins (42704) over the
-- colliding declared name (42P07). A different Postgres mechanism from the key-column
-- ordering next door, and it does not follow from it.
-- @setup
CREATE SEQUENCE idtype_taken;
-- @test
CREATE TABLE idtype (id nosuchtype GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME idtype_taken));
-- @expect-error: the unknown column type outranks the taken sequence name (42704)
