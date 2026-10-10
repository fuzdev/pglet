-- The correlated outer reference in a **`JOIN … ON` inside the subquery**. The
-- subquery's join tree is part of its own query level, so the outer relations chain
-- onto it exactly as they do onto its `WHERE`.
-- @setup
CREATE TABLE cj_o (okey INT, ouid UUID);
CREATE TABLE cj_i (ikey INT, itag TEXT);
INSERT INTO cj_o VALUES (1, '11111111-1111-1111-1111-111111111111'), (2, '22222222-2222-2222-2222-222222222222');
INSERT INTO cj_i VALUES (1, 'a');
-- @test
SELECT okey FROM cj_o WHERE EXISTS (SELECT 1 FROM cj_i a JOIN cj_i b ON a.ikey = b.ikey AND cj_o.ouid = '11111111-1111-1111-1111-111111111111') ORDER BY okey;
