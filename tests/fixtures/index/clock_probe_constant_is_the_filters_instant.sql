-- An index probe whose constant is `now()` and a row filter that evaluates `now()` per
-- row read one instant, so the indexed and the stripped build find the same rows: each
-- row this transaction stamped, by equality, by a range that ends at it, and by none
-- past it.
-- @profile-lock
-- @setup
CREATE TABLE cpc (id INT, at TIMESTAMPTZ DEFAULT now(), day DATE DEFAULT CURRENT_DATE);
CREATE INDEX cpc_at ON cpc (at);
CREATE INDEX cpc_day ON cpc (day);
INSERT INTO cpc (id) VALUES (1), (2), (3);
INSERT INTO cpc (id, at) VALUES (4, now() - INTERVAL '1 hour'), (5, now() + INTERVAL '1 hour');
-- @test
SELECT 'eq' AS probe, id FROM cpc WHERE at = now()
UNION ALL
SELECT 'eq and filter', id FROM cpc WHERE at = now() AND now() = CURRENT_TIMESTAMP
UNION ALL
SELECT 'le', id FROM cpc WHERE at <= now()
UNION ALL
SELECT 'gt', id FROM cpc WHERE at > now()
UNION ALL
SELECT 'between', id FROM cpc WHERE at BETWEEN now() AND transaction_timestamp()
UNION ALL
SELECT 'day', id FROM cpc WHERE day = CURRENT_DATE AND at = now()
ORDER BY 1, 2;
