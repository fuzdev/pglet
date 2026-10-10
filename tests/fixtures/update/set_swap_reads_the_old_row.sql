-- The swap idiom: each side reads the other's old value, so the two columns exchange
-- values. pglet answered `11 | 11`.
-- @setup
CREATE TABLE sosw (a INT, b INT);
INSERT INTO sosw VALUES (10, 11);

-- @test
UPDATE sosw SET a = b, b = a RETURNING a, b;
