-- The accept side's negative control: `DELETE … WHERE 'f'` removes nothing, on
-- both engines, before and after — so a resolver that answered TRUE for every
-- unresolved literal would redden here.
-- @setup
CREATE TABLE bcd (a INT);
INSERT INTO bcd VALUES (1), (2);
-- @test
DELETE FROM bcd WHERE 'off' RETURNING a;
