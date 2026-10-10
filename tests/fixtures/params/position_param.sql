-- A bound text param in both operands of POSITION(sub IN str): both positions are
-- text, so each bare $N must type as text over the extended protocol.
-- @setup
CREATE TABLE hay (id INT, s TEXT);
INSERT INTO hay VALUES (1, 'Thomas'), (2, 'other');
-- @param $1 text om
-- @test
SELECT id, position($1 IN s) AS pos FROM hay ORDER BY id;
