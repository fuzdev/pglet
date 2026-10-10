-- The accept-side pin for `order_by_member_position`: casting the enum **away** puts
-- the sort back on the label bytes, on both engines. A fix that ordered every `text`
-- comparison by some enum's members, or that read the enum through the cast, reddens
-- here while the member-order fixtures stay green.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id, m::text AS label FROM t ORDER BY m::text;
