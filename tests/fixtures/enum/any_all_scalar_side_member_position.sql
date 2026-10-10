-- The accept-side control for `any_all_array_side_member_position`: with the enum on the
-- **scalar** side the walk already read it, and both engines agreed before the array side
-- was widened. What it pins is that widening the array side did not move an answer that
-- was already right — not that it is independent of that side: the `AnyAll` arm now asks
-- `operand_can_be_enum` of **both** operands, so reverting the gate's array recursion
-- reddens this fixture too — measured; it is among the eight `EDGES.md` row 111 records
-- for that mutation.
--
-- 18.6 resolves this spelling at all only because the array's element type is the enum:
-- `m > ANY(ARRAY['ok'])` is `operator does not exist: mood > text` there, an all-`unknown`
-- array typing as `text[]`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'happy'), (2, 'sad'), (3, 'ok');

-- @test
SELECT id, (m > ANY(ARRAY['ok'::mood])) AS c, (m < ALL(ARRAY['ok'::mood])) AS d FROM t ORDER BY id;
