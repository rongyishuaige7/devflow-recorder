#!/usr/bin/env python3
"""
屏幕截图 OCR 测试脚本
依赖: tesseract-ocr, gnome-screenshot, Pillow
安装: 
  sudo apt install tesseract-ocr tesseract-ocr-chi-sim gnome-screenshot
用法:
  python3 tools/screenshot_ocr_test.py              # 全屏截图
  python3 tools/screenshot_ocr_test.py --region      # 区域截图
  python3 tools/screenshot_ocr_test.py --window      # 窗口截图
"""

import sys
import os
import argparse
import subprocess
import shutil

TMP_DIR = "/tmp"
SCREENSHOT_FILE = os.path.join(TMP_DIR, "screenshot_ocr_test.png")

def take_screenshot(output_path=None, region=False, window=False):
    """截取屏幕 via gnome-screenshot"""
    output_path = output_path or SCREENSHOT_FILE
    
    # 确保 tmp 目录存在
    os.makedirs(os.path.dirname(output_path), exist_ok=True)
    
    # 优先用 gnome-screenshot（系统级，不受 snap 隔离影响）
    if shutil.which("gnome-screenshot"):
        cmd = ["gnome-screenshot", "-f", output_path]
        if region:
            cmd.append("--area")
        elif window:
            cmd.append("--window")
        print(f"截图命令: {' '.join(cmd)}")
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
        if result.returncode == 0 and os.path.exists(output_path):
            print(f"截图已保存: {output_path}")
            return output_path
        else:
            print(f"截图失败: {result.stderr}")
            return None
    else:
        print("错误: 未找到 gnome-screenshot")
        return None

def ocr_tesseract(image_path, lang=None):
    """调用 tesseract OCR"""
    cmd = ["tesseract", image_path, "stdout"]
    if lang:
        cmd.extend(["-l", lang])
    
    print(f"\nOCR 识别中... (lang={lang or 'eng'})")
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
        if result.returncode == 0:
            return result.stdout
        else:
            print(f"OCR 错误: {result.stderr}")
            return None
    except subprocess.TimeoutExpired:
        print("OCR 超时")
        return None

def main():
    parser = argparse.ArgumentParser(description="屏幕截图 OCR 测试")
    parser.add_argument("--lang", "-l", default=None, help="语言，如 chi_sim(简体), eng")
    parser.add_argument("--region", "-r", action="store_true", help="区域截图（拖动选择）")
    parser.add_argument("--window", "-w", action="store_true", help="窗口截图（点击选择窗口）")
    parser.add_argument("--image", "-i", help="指定已有图片路径")
    args = parser.parse_args()
    
    # 截图或使用已有图片
    if args.image:
        image_path = args.image
        if not os.path.exists(image_path):
            print(f"图片不存在: {image_path}")
            sys.exit(1)
    else:
        image_path = take_screenshot(region=args.region, window=args.window)
    
    # OCR
    text = ocr_tesseract(image_path, lang=args.lang)
    
    if text:
        print("\n" + "="*50)
        print("OCR 结果:")
        print("="*50)
        print(text)
        print("="*50)
        
        # 保存结果
        result_file = os.path.join(TMP_DIR, "ocr_result.txt")
        with open(result_file, "w", encoding="utf-8") as f:
            f.write(text)
        print(f"\n结果已保存: {result_file}")
    else:
        print("OCR 识别失败")
        sys.exit(1)

if __name__ == "__main__":
    main()