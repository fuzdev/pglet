// Web Worker for persistent pglet database (B+tree storage). Loads the build with the
// crate's `opfs` feature on (`deno task wasm:build:opfs`): the default build exports no
// `PgletBtreePersistent`.
import init, { PgletBtreePersistent } from '../../crates/pglet_wasm/pkg-opfs/pglet_wasm.js';

let db = null;

async function initDB() {
    await init();
    console.log('[Worker] WASM initialized');

    try {
        db = await PgletBtreePersistent.open();
        console.log('[Worker] PgletBtreePersistent opened');

        // Ensure meta table exists for visit counter
        try {
            db.exec(`CREATE TABLE IF NOT EXISTS _pglet_meta (key TEXT, value TEXT)`);
        } catch (e) {
            // Table might already exist
        }

        return { success: true };
    } catch (e) {
        console.error('[Worker] Failed to open persistent DB:', e);
        return { success: false, error: String(e) };
    }
}

function executeSQL(sql) {
    if (!db) {
        return { error: 'Database not initialized' };
    }

    try {
        // Try query first (returns results)
        const result = db.query(sql);
        return {
            success: true,
            result: {
                rows: result.rows,
                rowCount: result.rowCount,
                fields: result.fields
            }
        };
    } catch (e) {
        // If query fails, try exec (for DDL/DML without results)
        try {
            db.exec(sql);
            return { success: true, result: { rows: [], rowCount: 0, fields: [] } };
        } catch (e2) {
            return { error: String(e2) };
        }
    }
}

function syncDB() {
    if (!db) {
        return { error: 'Database not initialized' };
    }
    try {
        db.sync();
        return { success: true };
    } catch (e) {
        return { error: String(e) };
    }
}

function clearDB() {
    if (!db) {
        return { error: 'Database not initialized' };
    }
    try {
        db.clear();
        return { success: true };
    } catch (e) {
        return { error: String(e) };
    }
}

function incrementVisit() {
    if (!db) {
        return { error: 'Database not initialized' };
    }

    try {
        // Get current visit count
        let currentCount = 0;
        try {
            const result = db.query(`SELECT value FROM _pglet_meta WHERE key = 'visit_count'`);
            if (result.rows && result.rows.length > 0) {
                currentCount = parseInt(result.rows[0].value, 10) || 0;
            }
        } catch (e) {
            // No existing count
        }

        const newCount = currentCount + 1;

        // Replace the visit count: `_pglet_meta` has no unique key, so an `ON CONFLICT (key)`
        // upsert would have no arbiter and be refused (`42P10`)
        db.exec(`DELETE FROM _pglet_meta WHERE key = 'visit_count'`);
        db.exec(`INSERT INTO _pglet_meta VALUES ('visit_count', '${newCount}')`);

        // Sync to ensure visit count persists
        db.sync();

        return { success: true, visitCount: newCount };
    } catch (e) {
        return { error: String(e) };
    }
}

function getStats() {
    if (!db) {
        return { error: 'Database not initialized' };
    }

    try {
        // Get visit count
        let visitCount = 0;
        try {
            const result = db.query(`SELECT value FROM _pglet_meta WHERE key = 'visit_count'`);
            if (result.rows && result.rows.length > 0) {
                visitCount = parseInt(result.rows[0].value, 10) || 0;
            }
        } catch (e) {
            // No visit count yet
        }

        // Get tables (excluding our meta table)
        let tables = [];
        let tableCount = 0;
        let totalRows = 0;

        try {
            // Simple query without JOINs - get all user tables
            const tablesResult = db.query(`
                SELECT relname AS table_name
                FROM pg_class
                WHERE relkind = 'r'
                ORDER BY relname
            `);

            if (tablesResult.rows) {
                for (const row of tablesResult.rows) {
                    const tableName = row.table_name;
                    if (tableName === '_pglet_meta') continue; // Skip internal table

                    let rowCount = 0;
                    try {
                        const countResult = db.query(`SELECT COUNT(*) AS cnt FROM "${tableName}"`);
                        if (countResult.rows && countResult.rows.length > 0) {
                            rowCount = parseInt(countResult.rows[0].cnt, 10) || 0;
                        }
                    } catch (e) {
                        // Couldn't count rows
                    }

                    tables.push({ name: tableName, rows: rowCount });
                    totalRows += rowCount;
                    tableCount++;
                }
            }
        } catch (e) {
            console.error('[Worker] Error getting tables:', e);
        }

        // Storage size - we can't easily get this from OPFS in the worker
        // For now, estimate based on row count (rough approximation)
        const storageBytes = totalRows * 100; // Very rough estimate: 100 bytes per row

        return {
            success: true,
            stats: {
                visitCount,
                tableCount,
                totalRows,
                storageBytes,
                tables
            }
        };
    } catch (e) {
        return { error: String(e) };
    }
}

// Handle messages from main thread
self.onmessage = async function(e) {
    const { id, type, sql } = e.data;

    let response;
    switch (type) {
        case 'init':
            response = await initDB();
            break;
        case 'exec':
            response = executeSQL(sql);
            break;
        case 'sync':
            response = syncDB();
            break;
        case 'clear':
            response = clearDB();
            break;
        case 'incrementVisit':
            response = incrementVisit();
            break;
        case 'stats':
            response = getStats();
            break;
        default:
            response = { error: `Unknown message type: ${type}` };
    }

    self.postMessage({ id, ...response });
};

console.log('[Worker] Worker loaded, ready for messages');
