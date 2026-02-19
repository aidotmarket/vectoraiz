import logging
from fastapi import APIRouter, Request, HTTPException
import hmac
import hashlib
import os
import json

router = APIRouter()

logger = logging.getLogger(__name__)

@router.post("/railway", summary="Railway Webhook", description="Receive deployment and health check events from Railway.")
async def railway_webhook(request: Request):
    try:
        payload = await request.body()
        signature = request.headers.get("X-Railway-Signature")
        secret = os.getenv("RAILWAY_WEBHOOK_SECRET")
        if secret:
            expected_sig = hmac.new(secret.encode('utf-8'), payload, hashlib.sha256).hexdigest()
            if not hmac.compare_digest(expected_sig, signature or ''):
                logger.warning("Invalid Railway webhook signature")
                raise HTTPException(status_code=401, detail="Invalid signature")
        else:
            logger.warning("RAILWAY_WEBHOOK_SECRET not set - skipping verification")

        data = json.loads(payload)
        event_type = data.get("type")
        logger.info(f"Received Railway event: type={event_type}, data={data}")

        # TODO: Handle specific events, e.g., publish to event bus, update status, or log to allAI

        return {"status": "ok"}
    except json.JSONDecodeError:
        raise HTTPException(status_code=400, detail="Invalid JSON")
    except Exception as e:
        logger.error(f"Error processing Railway webhook: {str(e)}")
        raise HTTPException(status_code=500, detail="Internal error")
