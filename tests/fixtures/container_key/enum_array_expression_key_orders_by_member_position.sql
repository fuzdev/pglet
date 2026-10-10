-- An enum array has no column form, so its one way into an index key is an expression,
-- `(ARRAY[m])`, whose key encodes each element's label bytes. That key is right for
-- equality and wrong for order — an enum orders by member position, and `happy` is the
-- last member and the first label — and nothing reads order through it: an expression
-- index serves no ordered scan and no range. The `ORDER BY` and the `>` here answer by
-- member position, index or no index.
-- @profile-lock
-- @setup
CREATE TYPE cke_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE cke_m (id INT, m cke_mood);
CREATE UNIQUE INDEX cke_m_u ON cke_m ((ARRAY[m]));
CREATE INDEX cke_m_i ON cke_m ((ARRAY[m]), id);
INSERT INTO cke_m (m, id) VALUES ('happy', 1), ('sad', 2), ('ok', 3);
-- @test
SELECT 'order' AS q, id FROM (SELECT id, row_number() OVER (ORDER BY ARRAY[m]) AS rn FROM cke_m) s WHERE rn = 1
UNION ALL SELECT 'above sad', id FROM cke_m WHERE ARRAY[m] > ARRAY['sad'::cke_mood]
UNION ALL SELECT 'equal ok', id FROM cke_m WHERE ARRAY[m] = ARRAY['ok'::cke_mood]
ORDER BY q, id;
