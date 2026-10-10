-- A set operation type-unifies its two sides' columns before anything is assigned, so
-- an all-`unknown` column resolves to `text` and 18.6 refuses the assignment — the
-- whole-list resolvers the walk deliberately does **not** descend, alongside a derived
-- table (`SELECT * FROM (VALUES ('t')) v`, `42804` on both engines too).
--
-- Read it against `assignment_cast_select_limit_does_not_resolve_the_unknown.sql`: both
-- put a node other than a projection on top of the source, and only one of them resolves
-- anything. That is what makes the fallback a rule rather than a leftover.
-- @setup
CREATE TABLE acso2 (id INT, b BOOL);

-- @expect-error: column "b" is of type boolean but expression is of type text (42804)
-- @test
INSERT INTO acso2 (b) SELECT 't' UNION SELECT 'f';
