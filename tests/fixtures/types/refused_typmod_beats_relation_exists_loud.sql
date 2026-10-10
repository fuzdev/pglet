-- `CREATE TABLE`'s `tableElts` walk precedes even the `42P07`, so a plain `CREATE TABLE`
-- over an existing relation answers the **type's** verdict and not the relation's. The
-- asymmetry with `ADD COLUMN` — where the column-existence check wins — is 18.6's, and
-- both sides of it are fixtures.
-- @setup
CREATE TABLE rtre (a INT);
-- @expect-error: the length refusal outranks the existing relation
-- @expect-message
-- @test
CREATE TABLE rtre (c CHAR(0));
