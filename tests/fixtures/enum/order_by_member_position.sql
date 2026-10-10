-- An enum orders by its members' **positions**, not its labels' bytes: `mood`'s
-- declared order is the exact reverse of its lexical one, so an `ORDER BY` over the
-- column and an `ORDER BY` over its text are opposite. The `AS label` is load-bearing:
-- an unaliased `m::text` is still named `m`, so the `ORDER BY` would bind that output
-- column and both engines would agree lexically, hiding the whole divergence.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id, m::text AS label FROM t ORDER BY m;
