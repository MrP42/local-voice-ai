#!/usr/bin/env python3
"""Install pinned public iPhone CPU dependencies. Never sends recordings or code."""
import argparse, hashlib, pathlib, shutil, tempfile, urllib.request, zipfile
ROOT = pathlib.Path(__file__).resolve().parents[1]
LIBRARIES = [
    ('whisper', 'https://github.com/ggml-org/whisper.cpp/releases/download/v1.7.5/whisper-v1.7.5-xcframework.zip', 'c7faeb328620d6012e130f3d705c51a6ea6c995605f2df50f6e1ad68c59c6c4a'),
    ('llama', 'https://github.com/ggml-org/llama.cpp/releases/download/b5046/llama-b5046-xcframework.zip', 'c19be78b5f00d8d29a25da41042cb7afa094cbf6280a225abe614b03b20029ab'),
]
MODELS = [
    ('ggml-base.bin', 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin', '60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe'),
    ('qwen2.5-0.5b-instruct-q4_k_m.gguf', 'https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf', '74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db'),
]
def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        while chunk := f.read(1024 * 1024): h.update(chunk)
    return h.hexdigest()
def fetch(url, path, expected):
    if path.exists() and digest(path) == expected: return
    temporary = path.with_suffix(path.suffix + '.download')
    urllib.request.urlretrieve(url, temporary)
    if digest(temporary) != expected:
        temporary.unlink()
        raise RuntimeError('SHA-256 mismatch: ' + path.name)
    temporary.replace(path)
def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--models', action='store_true', help='Also download 640 MB of local model weights')
    parser.add_argument('--small', action='store_true', help='Also download Whisper Small for the accuracy comparison')
    parser.add_argument('--meetings', action='store_true', help='Also download 1.12 GB Qwen weights for meeting analysis')
    args = parser.parse_args()
    vendor = ROOT / 'Vendor'; vendor.mkdir(exist_ok=True)
    for name, url, expected in LIBRARIES:
        archive = vendor / (name + '.zip'); fetch(url, archive, expected)
        destination = vendor / (name + '.xcframework')
        if not destination.exists():
            with tempfile.TemporaryDirectory(dir=vendor) as temp:
                with zipfile.ZipFile(archive) as zipped:
                    for entry in zipped.infolist():
                        if not (pathlib.Path(temp) / entry.filename).resolve().is_relative_to(pathlib.Path(temp).resolve()):
                            raise RuntimeError('Invalid archive path')
                    zipped.extractall(temp)
                shutil.move(str(pathlib.Path(temp) / 'build-apple' / (name + '.xcframework')), destination)
        print(name + ': pinned archive verified')
    if args.meetings:
        models = vendor / 'Models'; models.mkdir(exist_ok=True)
        fetch('https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf', models / 'qwen2.5-1.5b-instruct-q4_k_m.gguf', '6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e')
        print('qwen2.5-1.5b-instruct-q4_k_m.gguf: SHA-256 verified')
    if args.small:
        models = vendor / 'Models'; models.mkdir(exist_ok=True)
        fetch('https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin', models / 'ggml-small.bin', '1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b')
        print('ggml-small.bin: SHA-256 verified')
    if args.models:
        models = vendor / 'Models'; models.mkdir(exist_ok=True)
        for name, url, expected in MODELS:
            fetch(url, models / name, expected)
            print(name + ': SHA-256 verified')
if __name__ == '__main__': main()
