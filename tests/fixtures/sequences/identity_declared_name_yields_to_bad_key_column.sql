-- `transformIndexConstraints` resolves the key columns during parse analysis, which
-- precedes the before-list's CREATE SEQUENCEs — so the missing key column wins (42703)
-- over the colliding declared name (42P07).
-- @setup
CREATE SEQUENCE idkey_taken;
-- @test
CREATE TABLE idkey (id INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME idkey_taken), PRIMARY KEY (nosuch));
-- @expect-error: the missing key column outranks the taken sequence name (42703)
