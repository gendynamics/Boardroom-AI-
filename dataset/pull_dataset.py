"""Pull the dataset from Hugging Face:  python dataset/pull_dataset.py"""
import json, os
from datasets import load_dataset
from huggingface_hub import HfApi

repo = HfApi().whoami()["name"] + "/paypal-boardroom-ai"
ds = load_dataset(repo, split="train")
print(len(ds), "boardroom examples loaded from", repo)
takes = json.loads(ds[0]["messages"][2]["content"])
print("BRIEF:", ds[0]["messages"][1]["content"])
for seat in ("Strategist", "Marketer", "CFO", "Growth Lead"):
    print("\n[" + seat + "]", takes[seat])
here = os.path.dirname(os.path.abspath(__file__))
ds.to_json(os.path.join(here, "..", "public", "boardroom_dataset.jsonl"))
print("\nSaved to public/boardroom_dataset.jsonl")
