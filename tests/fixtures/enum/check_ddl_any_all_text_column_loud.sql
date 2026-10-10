-- The DDL-time twin of the `op ANY`/`op ALL` arm. A stored expression reaches this
-- position — a subquery does not, both engines refusing one in every persisted context —
-- so the array spelling is the half of the pair that had to land in **both** walks. What
-- it cost was a recorded `CHECK` whose dump stock 18.6 refuses to load while `psql`
-- exits `0`; that the catalog now holds nothing is `crates/pglet`'s
-- `enum_identity_gates`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok'), (2, 'ok', 'ok'), (3, 'happy', 'zzz');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
CREATE TABLE ck (m mood NOT NULL, s TEXT NOT NULL, CHECK (m = ANY(ARRAY[s])));
