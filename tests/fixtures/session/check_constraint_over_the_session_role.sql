-- 18.6 admits a **stable** function in a CHECK (immutability is the user's
-- responsibility there, unlike an index expression, which is `42P17`), so the
-- constraint is accepted and enforced on both engines. The insert that passes and
-- the one that is refused are both in the setup, so the `@test` reads what survived.
-- @setup
CREATE TABLE cc_owner (id INT, owner TEXT CHECK (owner = current_user));
INSERT INTO cc_owner VALUES (1, 'postgres');
-- @test
SELECT id, owner FROM cc_owner;
