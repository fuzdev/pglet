-- The other half of the `RETURNING` gap: without the coercion pass an unknown literal
-- never adopted its sibling's type, so `i4 = '10'` reached eval as `integer = text` and
-- failed loud (`42883`) on a query Postgres answers. Resolution and the gate travel
-- together here, exactly as they do for the `WHERE`.
-- @setup
CREATE TABLE url (id INT, i4 INT4);
INSERT INTO url VALUES (1, 10), (2, 20);
-- @test
UPDATE url SET id = id RETURNING id, i4 = '10' AS eq;
