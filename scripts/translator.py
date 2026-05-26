"""NLLB-200 translation worker for anuvaad.

Reads JSON requests from stdin, writes JSON responses to stdout.

Request:  {"id": 1, "text": "hello", "src": "eng_Latn", "tgt": "hin_Deva"}
Response: {"id": 1, "translation": "..."}  or  {"id": 1, "error": "..."}

On startup, writes one of:
  {"ready": true, "device": "cuda"|"cpu"}
  {"error": "..."}
"""

import json
import os
import sys
import traceback

CACHE_DIR = os.path.join(os.path.expanduser("~"), ".cache", "anuvaad")
MODEL_DIR = os.path.join(CACHE_DIR, "nllb-200-distilled-600M-ct2")
MODEL_REPO = "JustFrederik/nllb-200-distilled-600M-ct2-int8"
TOKENIZER_REPO = "facebook/nllb-200-distilled-600M"


def log(msg: str) -> None:
    sys.stderr.write(f"[translator] {msg}\n")
    sys.stderr.flush()


def emit(obj: dict) -> None:
    sys.stdout.write(json.dumps(obj, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def ensure_model() -> None:
    if os.path.isfile(os.path.join(MODEL_DIR, "model.bin")):
        return
    log(f"Downloading NLLB-200 (ct2 int8) to {MODEL_DIR} — first run only")
    from huggingface_hub import snapshot_download

    snapshot_download(repo_id=MODEL_REPO, local_dir=MODEL_DIR, local_dir_use_symlinks=False)


def main() -> None:
    try:
        import ctranslate2
        from transformers import AutoTokenizer
    except ImportError as e:
        emit(
            {
                "error": f"missing python dep: {e}. Run: pip install ctranslate2 transformers sentencepiece huggingface_hub"
            }
        )
        return

    try:
        os.makedirs(CACHE_DIR, exist_ok=True)
        ensure_model()
    except Exception as e:
        emit({"error": f"model fetch: {e}"})
        log(traceback.format_exc())
        return

    try:
        tokenizer = AutoTokenizer.from_pretrained(TOKENIZER_REPO)
    except Exception as e:
        emit({"error": f"tokenizer load: {e}"})
        log(traceback.format_exc())
        return

    def try_init(device: str, compute_type: str):
        t = ctranslate2.Translator(MODEL_DIR, device=device, compute_type=compute_type)
        # Warm-up: an actual translate call exposes runtime DLL mismatches that init alone hides.
        t.translate_batch(
            [["eng_Latn", "▁Hello"]],
            target_prefix=[["eng_Latn"]],
            beam_size=1,
            max_decoding_length=8,
        )
        return t

    translator = None
    device = "cpu"
    for attempt_device, attempt_ct in (("cuda", "int8_float16"), ("cpu", "int8")):
        try:
            translator = try_init(attempt_device, attempt_ct)
            device = attempt_device
            break
        except Exception as e:
            log(f"{attempt_device} init/warmup failed: {e}")
    if translator is None:
        emit({"error": "translator init failed on both CUDA and CPU"})
        log(traceback.format_exc())
        return

    emit({"ready": True, "device": device})

    for raw in sys.stdin:
        raw = raw.strip()
        if not raw:
            continue
        req_id = 0
        try:
            req = json.loads(raw)
            req_id = int(req.get("id", 0))
            text = req["text"]
            src = req["src"]
            tgt = req["tgt"]

            tokenizer.src_lang = src
            input_ids = tokenizer(text, return_tensors=None)["input_ids"]
            src_tokens = tokenizer.convert_ids_to_tokens(input_ids)

            results = translator.translate_batch(
                [src_tokens],
                target_prefix=[[tgt]],
                beam_size=1,
                max_decoding_length=256,
            )
            tgt_tokens = results[0].hypotheses[0]
            # Drop the leading target-language token
            if tgt_tokens and tgt_tokens[0] == tgt:
                tgt_tokens = tgt_tokens[1:]
            tgt_ids = tokenizer.convert_tokens_to_ids(tgt_tokens)
            out_text = tokenizer.decode(tgt_ids, skip_special_tokens=True)

            emit({"id": req_id, "translation": out_text})
        except Exception as e:
            log(f"translate error: {e}")
            log(traceback.format_exc())
            emit({"id": req_id, "error": str(e)})


if __name__ == "__main__":
    main()
