-- The enum half of the same rule: a `Cast` erases an enum's identity so the pass leaves
-- the branch bare, and the gate reads its type and refuses the set. The gate's verdict
-- is read through a `ResolvedType`, so the **type name** in the message is now the enum's
-- rather than its base `text` (`EDGES.md` row 152's enum member, closed with row 114).
-- It still cannot carry `@expect-message`: every `Error::Type` sentence is prefixed
-- `type error:`, which is the whole `42804` family's divergence and not this one's.
-- @setup
CREATE TYPE ssmood AS ENUM ('sad', 'ok');
CREATE TABLE ssem (m ssmood);
INSERT INTO ssem VALUES ('ok');
CREATE TABLE ssel (i INT, d DATE);
INSERT INTO ssel VALUES (1, '2020-01-02');
-- @test
SELECT COALESCE((SELECT m FROM ssem), i, d) FROM ssel;
-- @expect-error: COALESCE types ssmood and integer cannot be matched
