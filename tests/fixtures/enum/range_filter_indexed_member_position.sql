-- The range probe's half of `order_by_indexed_member_position`: `m > 'sad'` over an
-- indexed enum column. The range probe fires with no `NOT NULL` gate at all, so this
-- reaches the index by a different route than the ordered scan does. See that fixture
-- for why neither carries `@profile-lock`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
CREATE INDEX t_m_idx ON t (m);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id FROM t WHERE m > 'sad' ORDER BY id;
